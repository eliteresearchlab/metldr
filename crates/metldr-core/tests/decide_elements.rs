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
                parent: c[3].as_str().unwrap(),
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
fn shortlist_ranks_outline_controls_for_the_goal() {
    let outline = "- banner:\n  - link \"Home\" [ref=e1]\n  - navigation \"Main\":\n    - link \"Pricing\" [ref=e2]\n    - link \"Docs\" [ref=e3]\n- main:\n  - heading \"Order 4471\" [level=1]\n  - button \"Apply update\" [ref=e4]\n  - button \"Cancel order\" [ref=e5]\n  - textbox \"Say \\\"hi\\\"\" [ref=e6]";
    let list = metldr_core::decide::elements::shortlist(outline, "Click Apply update", &[], 2);
    let lines: Vec<&str> = list.lines().collect();
    assert_eq!(lines[0], "2 of 6 controls, best first for the goal; snapshot without goal for the whole page.");
    assert_eq!(lines[1], "- button \"Apply update\" [ref=e4]");
    assert_eq!(lines.len(), 3);
}
