//! The port scores saved Mind2Web steps (unseen sites) as the trained scorer did.
#[test]
fn port_agrees_with_the_trained_scorer() {
    let cases: serde_json::Value = serde_json::from_str(include_str!("elements_check.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let candidates = case["candidates"].as_array().unwrap();
        let controls: Vec<metldr_core::decide::elements::Control> = candidates
            .iter()
            .map(|c| metldr_core::decide::elements::Control {
                role: c[0].as_str().unwrap(),
                name: c[1].as_str().unwrap(),
                clickable: c[2].as_bool().unwrap(),
            })
            .collect();
        let goal = case["goal"].as_str().unwrap();
        // The trained steps hold the previous actions already joined, newest first.
        let previous = case["previous_text"].as_str().unwrap();
        let got = metldr_core::decide::elements::scores(goal, &[previous], &controls);
        for (got, want) in got.iter().zip(case["scores"].as_array().unwrap()) {
            let want = want.as_f64().unwrap() as f32;
            assert!((got - want).abs() < 1e-4, "{got} != {want}");
        }
    }
}

#[test]
fn the_scout_ranks_the_named_control_first_on_a_real_outline() {
    // A Browse outline nests a section's controls under its heading line; the scout must not
    // read that as the button sitting inside a heading.
    let outline = include_str!("elements_outline.txt");
    let ranked = metldr_core::decide::elements::ranked(outline, "Click Apply update", &[]);
    let mut list = String::new();
    metldr_core::decide::elements::write_shortlist(&ranked[..3], ranked.len(), "", &mut list);
    let lines: Vec<&str> = list.lines().collect();
    assert_eq!(lines[0], "3 of 11 controls, best first for the goal; snapshot without goal for the whole page.");
    assert_eq!(lines[1], "- button \"Apply update\" [ref=e11]");
    assert_eq!(lines.len(), 4);
}
