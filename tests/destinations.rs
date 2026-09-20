use tutor::{detect_destinations, format_destinations_markdown};

#[test]
fn detects_agents_destinations() {
    let destinations = detect_destinations(None);

    let project_agents = destinations.iter().find(|d| d.id == "project_agents");
    assert!(project_agents.is_some());
    let pa = project_agents.unwrap();
    assert!(pa.label.contains("AGENTS.md"));

    let global_agents = destinations.iter().find(|d| d.id == "global_agents");
    assert!(global_agents.is_some());
    let ga = global_agents.unwrap();
    assert!(ga.label.contains("AGENTS.md"));
}

#[test]
fn formats_destinations_markdown_includes_agents() {
    let destinations = detect_destinations(None);
    let md = format_destinations_markdown(&destinations);

    assert!(md.contains("`project_agents`"));
    assert!(md.contains("<cwd>/AGENTS.md"));
    assert!(md.contains("`global_agents`"));
    assert!(md.contains("~/.config/rules/AGENTS.md"));
}
