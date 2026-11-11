//! Routes files: valid ones resolve URL paths to the files they name, and invalid ones are
//! refused.

use sekai_serve::{Routes, Target};

fn routes(json: &str) -> Routes {
    Routes::from_json(json.as_bytes()).unwrap()
}

fn sample() -> Routes {
    routes(
        r#"{
            "routes": [
                { "pattern": "/files/{region}-{view}/{path}" },
                { "pattern": "/jp-storage/{path}", "region": "jp", "view": "storage" },
                { "pattern": "/{region}@{version}/{path}", "view": "canonical" },
                { "pattern": "/{region}/{path}", "view": "canonical" }
            ]
        }"#,
    )
}

fn target(region: &str, view: &str, version: Option<&str>, path: &str) -> Option<Target> {
    Some(Target {
        region: region.to_owned(),
        view: view.to_owned(),
        version: version.map(str::to_owned),
        path: path.to_owned(),
    })
}

#[test]
fn the_first_route_that_matches_decides() {
    let routes = sample();
    let cases = [
        (
            "/jp/music/0001.mp3",
            target("jp", "canonical", None, "music/0001.mp3"),
        ),
        (
            "/jp@6.1.0/a.json",
            target("jp", "canonical", Some("6.1.0"), "a.json"),
        ),
        (
            "/files/en-storage/x/y.png",
            target("en", "storage", None, "x/y.png"),
        ),
        (
            "/jp-storage/sprite/a.png",
            target("jp", "storage", None, "sprite/a.png"),
        ),
        // `en` is not `<region>-<view>`, so a later route takes the path.
        (
            "/files/en/x.png",
            target("files", "canonical", None, "en/x.png"),
        ),
    ];
    for (path, expected) in cases {
        assert_eq!(routes.resolve(path), expected, "{path}");
    }
}

#[test]
fn paths_are_percent_decoded() {
    let routes = sample();
    assert_eq!(
        routes.resolve("/jp/sprite/%E9%9D%A2%E6%9D%BF/a%20b.png"),
        target("jp", "canonical", None, "sprite/面板/a b.png")
    );
    assert_eq!(
        routes.resolve("/jp/a%2Fb"),
        target("jp", "canonical", None, "a/b")
    );
    for broken in ["/jp/a%2", "/jp/a%zz", "/jp/%FF"] {
        assert_eq!(routes.resolve(broken), None, "{broken}");
    }
}

#[test]
fn names_that_cannot_name_a_file_match_nothing() {
    let routes = sample();
    for path in [
        "",
        "jp/a.png",
        "/jp",
        "/jp/",
        "/../a.png",
        "/./a.png",
        "/j p/a.png",
        "/jp@/a.png",
        "/jp@../a.png",
    ] {
        assert_eq!(routes.resolve(path), None, "{path}");
    }
}

#[test]
fn names_of_devices_match_nothing() {
    let routes = sample();
    let devices = [
        "con", "NUL", "Aux", "prn", "com1", "LPT9", "nul.json", "con.1.2", "COM0.x", "nul.",
    ];
    for name in devices {
        for path in [format!("/{name}/a.png"), format!("/jp@{name}/a.png")] {
            assert_eq!(routes.resolve(&path), None, "{path}");
        }
    }
    // A name that only begins or ends like one is a name.
    for name in [
        "console", "com", "com10", "lpt", "nulls", "a.con", "con-1", "_nul",
    ] {
        assert_eq!(
            routes.resolve(&format!("/{name}/a.png")),
            target(name, "canonical", None, "a.png")
        );
        assert_eq!(
            routes.resolve(&format!("/jp@{name}/a.png")),
            target("jp", "canonical", Some(name), "a.png")
        );
    }
}

#[test]
fn a_target_names_its_pointer() {
    let current = target("jp", "storage", None, "a").unwrap();
    assert_eq!(current.pointer(), "jp/storage/current.json");
    let pinned = target("jp", "canonical", Some("6.1.0"), "a").unwrap();
    assert_eq!(pinned.pointer(), "jp/canonical/versions/6.1.0.json");
}

#[test]
fn defaults_and_settings_are_read() {
    let plain = sample();
    assert_eq!(plain.max_age(), 60);
    let set = routes(
        r#"{ "routes": [{ "pattern": "/{region}/{view}/{path}" }],
             "delivery": "redirect", "store_url": "https://cdn.example/store/", "max_age": 5 }"#,
    );
    assert_eq!(set.max_age(), 5);
}

#[test]
fn invalid_routes_are_refused() {
    let one = |route: &str| format!(r#"{{ "routes": [{route}] }}"#);
    let cases = [
        r#"{ "routes": [] }"#.to_owned(),
        one(r#"{ "pattern": "{region}/{view}/{path}" }"#),
        one(r#"{ "pattern": "/{region}/{view}" }"#),
        one(r#"{ "pattern": "/{region}/{view}/x{path}" }"#),
        one(r#"{ "pattern": "/{path}/{region}/{view}/{path}" }"#),
        one(r#"{ "pattern": "/{region}{view}/{path}" }"#),
        one(r#"{ "pattern": "/{region}/{region}/{view}/{path}" }"#),
        one(r#"{ "pattern": "/{region}/{shard}/{view}/{path}" }"#),
        one(r#"{ "pattern": "/{region/{view}/{path}" }"#),
        one(r#"{ "pattern": "/region}/{view}/{path}" }"#),
        one(r#"{ "pattern": "/{region}//{view}/{path}" }"#),
        one(r#"{ "pattern": "/{region}/{path}" }"#),
        one(r#"{ "pattern": "/{region}/{view}/{path}", "view": "storage" }"#),
        one(r#"{ "pattern": "/{path}", "region": "jp", "view": ".." }"#),
        one(r#"{ "pattern": "/{path}", "region": "nul", "view": "storage" }"#),
        one(r#"{ "pattern": "/{region}/{view}/{path}", "shard": "x" }"#),
        r#"{ "routes": [{ "pattern": "/{region}/{view}/{path}" }], "delivery": "redirect" }"#
            .to_owned(),
        r#"{ "routes": [{ "pattern": "/{region}/{view}/{path}" }], "store_url": "https://a" }"#
            .to_owned(),
        r#"{ "routes": [{ "pattern": "/{region}/{view}/{path}" }], "cors": "a\nb" }"#.to_owned(),
        r#"{ "routes": [{ "pattern": "/{region}/{view}/{path}" }], "extra": 1 }"#.to_owned(),
    ];
    for case in cases {
        assert!(Routes::from_json(case.as_bytes()).is_err(), "{case}");
    }
}
