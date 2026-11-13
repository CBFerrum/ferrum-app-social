//! The connection descriptions under docs/connectors, as the platform takes them and as the package ships them.
//!
//! What each network's description does is proved against a fake of its documented API in `networks_a_it.rs` and
//! `networks_b_it.rs`, and Social's own behaviour through them in `social_it.rs`; this file proves what holds for all of
//! them: each is a description the platform takes, each network says what it takes and asks (`meta.social`), each
//! inbox mapping writes as the system, and the package ships exactly the files.

use ferrum_kernel::connections::Description;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/connectors")
}

/// Every description file: the networks, then the services Social calls outside an account.
fn descriptions() -> Vec<(String, PathBuf, bool)> {
    let mut out = vec![];
    for (sub, network) in [("", true), ("services", false)] {
        for entry in std::fs::read_dir(dir().join(sub)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "json") {
                out.push((path.file_stem().unwrap().to_string_lossy().to_string(), path, network));
            }
        }
    }
    out.sort();
    out
}

/// Run on a thread with a large stack: the platform's parsing of the larger descriptions (Tumblr's fourteen operations)
/// goes deeper than a test thread's 2 MB (a platform item: a server worker saving one must not do the same).
fn deep<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(16 << 20).spawn(f).unwrap().join().unwrap()
}

#[test]
fn every_description_is_one_the_platform_takes() {
    deep(|| {
        let all = descriptions();
        assert!(all.iter().filter(|d| d.2).count() >= 30, "the networks: {}", all.len());
        for (name, path, network) in all {
            let text = std::fs::read_to_string(&path).unwrap();
            let d: Description = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(d.problem(), None, "{name}");
            assert!(d.apps.iter().any(|a| a == "social"), "{name} lists social under apps");
            let raw: Value = serde_json::from_str(&text).unwrap();
            if network {
                assert!(raw["meta"]["social"].is_object(), "{name} says what its network takes and asks (meta.social)");
                assert!(d.operations.contains_key("publish"), "{name} publishes");
            }
            // Every inbox mapping writes as the system: a message's network id is read-only (BD-58).
            let maps = raw["pulls"].as_array().into_iter().flatten().map(|p| &p["map"]).chain(std::iter::once(&raw["inbound"]["map"]));
            for m in maps.filter(|m| m.is_object()) {
                assert_eq!(m["as"], "system", "{name}: its mapping onto {} writes as the system", m["entity"]);
            }
        }
    });
}

#[test]
fn the_package_ships_every_description_as_the_file_is() {
    deep(|| {
        let packages = ferrum_app_social::packages().unwrap();
        let shipped = &packages.iter().find(|p| p.manifest.package == "social").unwrap().manifest.connections;
        let mut names: Vec<String> = shipped.iter().filter_map(|c| c["name"].as_str().map(str::to_string)).collect();
        names.sort();
        let mut files: Vec<String> = descriptions().into_iter().map(|d| d.0).collect();
        files.sort();
        assert_eq!(names, files, "every description is offered to a workspace: run scripts/ship_connections.py");
        for c in shipped {
            let name = c["name"].as_str().unwrap();
            let path = descriptions().into_iter().find(|d| d.0 == name).unwrap().1;
            let file: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            assert_eq!(c["description"], file, "{name}: the package's copy is the file (run scripts/ship_connections.py)");
        }
    });
}
