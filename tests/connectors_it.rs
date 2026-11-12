//! The example connection descriptions under docs/connectors load and validate as the platform's descriptions
//! (shape only: nothing here calls a network).

use ferrum_kernel::connections::Description;

#[test]
fn the_examples_are_descriptions_the_platform_takes() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/connectors");
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let d: Description = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(d.problem(), None, "{}", path.display());
        assert!(d.apps.iter().any(|a| a == "social") && d.operations.contains_key("publish"), "{}", path.display());
        seen += 1;
    }
    assert_eq!(seen, 2);
}
