//! Incremental Mixer Brush preview, with the same dabs and mixing as the command.

use std::sync::Arc;

use photocraft_doc::{Document, LayerId};
use photocraft_paint::mixer::MixerDab;
use photocraft_paint::retouch::{Region, mix_back};

use super::*;

/// Mixer Brush pixels and carried paint while a gesture is in progress.
pub struct LiveMixer {
    /// Document containing the current preview, without changing session history.
    pub doc: Arc<Document>,
    /// Jitter seed to record with the committed command.
    pub seed: u64,
    pre_doc: Arc<Document>,
    pre: Surface,
    work: Surface,
    renderer: StrokeRenderer,
    mixer: MixerDab,
    settings: MixerSettings,
    brush: BrushSettings,
    initial: MixerState,
    layer: Option<LayerId>,
    params: Value,
    points: Vec<StrokePoint>,
    base: (photocraft_doc::DocId, u64),
    selection: Option<Surface>,
    lock: bool,
    color_channel: Option<usize>,
    done: usize,
    bounds: Rect,
    tail: Rect,
}

impl LiveMixer {
    /// Start a preview using the same parameters as `paint.mixerBrush`.
    pub fn begin(s: &Session, p: &Value) -> Result<Self> {
        has_paintable(s).map_err(EngineError::Other)?;
        let (stroke, settings, initial, layer) = mixer_setup(s, p)?;
        let active = s.active().ok_or(EngineError::NoDocument)?;
        let mut doc = (*active.doc).clone();
        let selection = doc.selection.clone();
        let (surface, lock) = crate::channel_cmds::target_surface(&mut doc, layer, p)?;
        let pre = surface.clone();
        // Keep working pixels at float precision between dabs, as apply_dab_stroke's Region
        // does. Quantizing every dab into an 8/16-bit Surface would change the final blend.
        let mut format = pre.format();
        format.sample = photocraft_color::SampleType::F32;
        let work = Surface::with_default(format, &pre.default_pixel());
        let mixer = MixerDab::new(pre.format(), &settings, &initial, lock);
        let mut dab_brush = stroke.brush.clone();
        dab_brush.opacity = 1.0;
        let mut live = Self {
            doc: Arc::new(doc),
            pre_doc: active.doc.clone(),
            pre,
            work,
            renderer: StrokeRenderer::new(&dab_brush, None, 1.0).record_dabs(),
            mixer,
            settings,
            seed: stroke.brush.seed,
            brush: stroke.brush,
            initial,
            layer,
            params: p.clone(),
            points: Vec::new(),
            base: (active.doc.id, active.revision),
            selection,
            lock,
            color_channel: crate::channel_cmds::color_restriction(s, "paint.mixerBrush", p),
            done: 0,
            bounds: Rect::EMPTY,
            tail: Rect::EMPTY,
        };
        live.push(&stroke.points)?;
        Ok(live)
    }

    /// Rectangle touched so far, including the finishing dab shown at the pointer.
    pub fn bounds(&self) -> Rect {
        self.bounds.union(&self.tail)
    }

    /// Whether airbrush build-up or smoothing still needs time while held.
    pub fn wants_time(&self) -> bool {
        self.renderer.wants_time()
    }

    fn sample(&self, rect: Rect) -> Option<Surface> {
        if !self.settings.sample_all_layers || rect.is_empty() {
            return None;
        }
        let buf = photocraft_compose::render(&self.pre_doc, rect);
        let mut sample = Surface::new(photocraft_color::PixelFormat::RGBA32F);
        sample.write_region(rect, &buf.px.iter().flatten().copied().collect::<Vec<_>>());
        Some(sample)
    }

    fn working_region(&self, rect: Rect) -> Region {
        let mut region = Region::read(&self.pre, rect);
        for key in rect.tiles().filter(|key| self.work.tile(*key).is_some()) {
            let touched = key.rect().intersect(&rect);
            let work = Region::read(&self.work, touched);
            for y in touched.y0..touched.y1 {
                for x in touched.x0..touched.x1 {
                    region.px_mut(x, y).copy_from_slice(work.px(x, y));
                }
            }
        }
        region
    }

    fn draw_pending(&mut self, show_tail: bool) -> Result<Rect> {
        let ctx = self.renderer.ctx.clone();
        let dabs = self.renderer.dabs().get(self.done..).unwrap_or_default().to_vec();
        let dirty = dabs.iter().fold(Rect::EMPTY, |r, d| r.union(&ctx.dab_rect(d, false)));
        Self::check_region(dirty)?;
        let sample = self.sample(dirty);
        for (i, dab) in dabs.iter().enumerate() {
            let r = ctx.dab_rect(dab, false);
            if r.is_empty() {
                continue;
            }
            // Initialize just this dab's new tiles, retaining float precision on overlaps.
            for key in r.tiles() {
                if self.work.tile(key).is_none() && self.pre.tile(key).is_some() {
                    let tile = key.rect();
                    self.work.write_region(tile, &self.pre.read_region(tile));
                }
            }
            let mut region = Region::read(&self.work, r);
            self.mixer.apply(&mut region, &ctx.footprint(dab, self.done + i), sample.as_ref());
            region.write(&mut self.work);
        }
        self.done += dabs.len();
        self.bounds = self.bounds.union(&dirty);
        let old_tail = std::mem::replace(&mut self.tail, Rect::EMPTY);
        let changed = dirty.union(&old_tail);
        Self::check_region(changed)?;
        let tail_dabs = if show_tail { self.renderer.tail_dabs() } else { Vec::new() };
        let tail = tail_dabs.iter().fold(Rect::EMPTY, |r, d| r.union(&ctx.dab_rect(d, false)));
        Self::check_region(tail)?;
        let mut tail_work = None;
        if !tail.is_empty() {
            // Finishing the stroke is speculative while drawing: neither reservoir depletion
            // nor pickup from this tail may leak into the next input batch.
            let mut region = self.working_region(tail);
            let mut mixer = self.mixer.clone();
            let sample = self.sample(tail);
            for (i, dab) in tail_dabs.iter().enumerate() {
                mixer.apply(&mut region, &ctx.footprint(dab, self.done + i), sample.as_ref());
            }
            tail_work = Some(region);
            self.tail = tail;
        }
        let working = (!changed.is_empty()).then(|| self.working_region(changed));
        let (surface, _) = crate::channel_cmds::target_surface(Arc::make_mut(&mut self.doc), self.layer, &self.params)?;
        if !changed.is_empty() {
            let work = working.ok_or_else(|| bad("paint.mixerBrush", "missing working pixels"))?;
            mix_back(surface, &Region::read(&self.pre, changed), &work, changed, 1.0, self.selection.as_ref(), self.lock);
        }
        if let Some(work) = tail_work {
            mix_back(surface, &Region::read(&self.pre, tail), &work, tail, 1.0, self.selection.as_ref(), self.lock);
        }
        let changed = changed.union(&tail);
        if let Some(channel) = self.color_channel.filter(|_| !changed.is_empty()) {
            Self::check_region(changed)?;
            let n = surface.channels().max(1);
            let before = self.pre.read_region(changed);
            let mut shown = surface.read_region(changed);
            for (new, old) in shown.chunks_exact_mut(n).zip(before.chunks_exact(n)) {
                for (k, (new, old)) in new.iter_mut().zip(old).enumerate() {
                    if k != channel {
                        *new = *old;
                    }
                }
            }
            surface.write_region(changed, &shown);
        }
        Ok(changed)
    }

    fn check_region(rect: Rect) -> Result<()> {
        // Bound input-derived temporary windows before Region or compose allocates. A long
        // diagonal pointer jump must not request a document-sized dense buffer without a cap.
        let pixels = u64::from(rect.width()).checked_mul(u64::from(rect.height()));
        if pixels.is_none_or(|pixels| pixels > 16 * 1024 * 1024) {
            return Err(bad("paint.mixerBrush", "live Mixer Brush window exceeds 16 million pixels; use a smaller brush or shorter input batches"));
        }
        Ok(())
    }

    /// Render a batch of pointer samples, rejecting invalid input before changing the preview.
    pub fn push(&mut self, points: &[StrokePoint]) -> Result<Rect> {
        check_coords(points, "paint.mixerBrush")?;
        self.points.extend_from_slice(points);
        self.renderer.push(points);
        self.draw_pending(true)
    }

    pub(crate) fn into_commit(mut self, s: &Session) -> Result<LiveCommit> {
        let mut p = self.params.clone();
        p["seed"] = json!(self.seed);
        let (stroke, settings, initial, layer) = mixer_setup(s, &p)?;
        if stroke.brush != self.brush
            || settings != self.settings
            || initial != self.initial
            || layer != self.layer
            || crate::channel_cmds::color_restriction(s, "paint.mixerBrush", &p) != self.color_channel
        {
            return Err(bad("paint.mixerBrush", "Mixer Brush settings changed during the stroke"));
        }
        self.renderer.finish();
        self.draw_pending(false)?;
        let (surface, _) = crate::channel_cmds::target_surface(Arc::make_mut(&mut self.doc), self.layer, &self.params)?;
        Ok(LiveCommit {
            cmd: "paint.mixerBrush".into(),
            base: self.base,
            params: self.params,
            seed: Some(self.seed),
            points: self.points,
            surface: surface.clone(),
            bounds: self.bounds,
            mixer: Some(self.mixer.state()),
        })
    }
}
