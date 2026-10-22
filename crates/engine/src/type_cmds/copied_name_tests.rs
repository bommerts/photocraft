// Included in type_cmds::tests: exercise the command seam, not a suffix parser.
fn copied_text(s: &mut Session, id: u64) -> u64 {
    s.execute("layer.duplicate", json!({"layer": id})).unwrap()["layer"].as_u64().unwrap()
}

#[test]
fn copied_auto_name_follows_edits_and_history() {
    let mut s = session();
    let original = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let first = copied_text(&mut s, original);
    let second = copied_text(&mut s, first);
    assert_eq!(layer_name_of(&s, first), "Heading copy");
    assert_eq!(layer_name_of(&s, second), "Heading copy 2");
    let steps = s.active().unwrap().history.entries().len();
    s.execute("type.edit", json!({"layer": second, "text": "New heading"})).unwrap();
    assert_eq!(layer_name_of(&s, second), "New heading");
    assert_eq!(layer_name_of(&s, original), "Heading");
    assert_eq!(layer_name_of(&s, first), "Heading copy");
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    assert!(s.undo());
    assert_eq!(layer_name_of(&s, second), "Heading copy 2");
    assert_eq!(text_layer(&s, second).text, "Heading");
    assert!(s.redo());
    assert_eq!(layer_name_of(&s, second), "New heading");
    assert_eq!(text_layer(&s, second).text, "New heading");
    assert!(s.undo());
    s.execute("type.edit", json!({"layer": second, "replace": {"start": 0, "end": 7, "text": "Again"}})).unwrap();
    assert_eq!(layer_name_of(&s, second), "Again");
}

#[test]
fn copied_auto_name_survives_non_text_edits_and_lorem_uses_same_policy() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let copy = copied_text(&mut s, id);
    s.execute("type.edit", json!({"layer": copy, "size": 20})).unwrap();
    s.execute("type.convertToParagraphText", json!({"layer": copy})).unwrap();
    assert_eq!(layer_name_of(&s, copy), "Heading copy", "formatting must not consume the copy name");
    s.execute("type.pasteLoremIpsum", json!({"layer": copy, "at": 0})).unwrap();
    assert_eq!(layer_name_of(&s, copy), layer_name(&text_layer(&s, copy).text));
    assert!(s.undo());
    assert_eq!(layer_name_of(&s, copy), "Heading copy");
    assert!(s.redo());
    assert_eq!(layer_name_of(&s, copy), layer_name(&text_layer(&s, copy).text));
}

#[test]
fn copied_custom_names_are_not_inferred_from_copy_suffixes() {
    for custom in ["Logo", "Heading copy", "Heading copy 2"] {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": "Heading", "name": custom})).unwrap()["layer"].as_u64().unwrap();
        let first = copied_text(&mut s, id);
        let second = copied_text(&mut s, first);
        let name = layer_name_of(&s, second);
        s.execute("type.edit", json!({"layer": second, "text": "Changed"})).unwrap();
        s.execute("type.pasteLoremIpsum", json!({"layer": second, "at": 0})).unwrap();
        assert_eq!(layer_name_of(&s, second), name);
    }
}

#[test]
fn copied_auto_name_can_be_explicitly_customized() {
    for command in ["layer.renameLayer", "layer.setProps", "type.edit"] {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
        let copy = copied_text(&mut s, id);
        // Even choosing exactly the generated name is an explicit custom label.
        s.execute(command, json!({"layer": copy, "name": "Heading copy"})).unwrap();
        s.execute("type.edit", json!({"layer": copy, "text": "Changed"})).unwrap();
        assert_eq!(layer_name_of(&s, copy), "Heading copy", "{command}");
        let again = copied_text(&mut s, copy);
        let name = layer_name_of(&s, again);
        s.execute("type.edit", json!({"layer": again, "text": "Changed again"})).unwrap();
        assert_eq!(layer_name_of(&s, again), name);
    }
}

#[test]
fn copied_auto_name_survives_native_save_and_reopen() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let copy = copied_text(&mut s, id);
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &photocraft_format::SaveOptions::default()).unwrap();
    let doc = photocraft_format::load_from_bytes(&bytes).unwrap();
    s.active_mut().unwrap().doc = Arc::new(doc);
    s.execute("type.edit", json!({"layer": copy, "text": "Reopened"})).unwrap();
    assert_eq!(layer_name_of(&s, copy), "Reopened");
}

#[test]
fn copied_auto_name_with_explicit_duplicate_label_stays_custom() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Heading"})).unwrap()["layer"].as_u64().unwrap();
    let copy = s.execute("layer.duplicate", json!({"layer": id, "name": "Heading copy"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.edit", json!({"layer": copy, "text": "Changed"})).unwrap();
    assert_eq!(layer_name_of(&s, copy), "Heading copy");
}

#[test]
fn copied_auto_names_use_the_normal_unicode_blank_and_length_rules() {
    for initial in ["", "\n  \nGröße ✓\nsecond", "A very long heading that exceeds forty characters easily", "Heading copy 2"] {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": initial})).unwrap()["layer"].as_u64().unwrap();
        let copy = copied_text(&mut s, id);
        s.execute("type.edit", json!({"layer": copy, "text": "\n 新しい見出し\nsecond"})).unwrap();
        assert_eq!(layer_name_of(&s, copy), "新しい見出し");
        s.execute("type.edit", json!({"layer": copy, "text": ""})).unwrap();
        assert_eq!(layer_name_of(&s, copy), "Type Layer");
    }
}
