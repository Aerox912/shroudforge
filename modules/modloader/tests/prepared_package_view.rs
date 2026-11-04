use camino::{Utf8Path as Path, Utf8PathBuf as PathBuf};
use shroudforge_package::{
    FileSystem, ModEnvironment,
    prepared_package_view::{
        PreparedPackageEntry as Entry, PreparedPackageView as View, PreparedReadBudget as Budget,
        PreparedSource as Source,
    },
    read_manifest, read_prepared_manifest,
};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    sync::Arc,
};

fn budget(bytes: u64, calls: u64) -> Budget {
    Budget::new(bytes, calls, 0, 0).unwrap()
}
fn policy() -> shroudforge_package::PreparedManifestPolicy {
    shroudforge_package::PreparedManifestPolicy {
        allow_unknown_manifest_fields: true,
    }
}
fn dir(path: &str) -> Entry {
    Entry {
        path: path.into(),
        source: None,
    }
}
fn file(path: &str, bytes: &[u8]) -> Entry {
    Entry {
        path: path.into(),
        source: Some(Source::bytes(Arc::from(bytes))),
    }
}
fn view(entries: Vec<Entry>, budget: Budget) -> View {
    View::new(
        PathBuf::from("E:/prepared-fixture/mods/example"),
        entries,
        budget,
    )
    .unwrap()
}
fn contents(fs: &mut FileSystem, path: &str) -> String {
    let mut s = String::new();
    fs.read_file(path).unwrap().read_to_string(&mut s).unwrap();
    s
}
fn manifest(id: &str) -> Vec<u8> {
    serde_json::to_vec(
        &serde_json::json!({"id":id,"name":"Example","version":"1.0.0","capabilities":["runtime"]}),
    )
    .unwrap()
}
fn extension() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"schemaVersion":1,"enabled":true,"settings":{"enabledFeature":{"value":true,"label":"Enabled","control":"toggle"}}})).unwrap()
}

#[test]
fn immutable_namespace_preserves_order_and_file_directory_truth() {
    let mut fs = FileSystem::new_prepared(view(
        vec![
            dir(""),
            file("z.txt", b"z"),
            dir("src"),
            file("src/profile.lua", b"new"),
            file("a.txt", b"a"),
        ],
        budget(100, 10),
    ));
    assert!(fs.exists("src/profile.lua") && fs.is_file("src/profile.lua"));
    assert!(!fs.is_directory("src/profile.lua") && fs.is_directory("src"));
    assert_eq!(
        fs.files()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        ["", "z.txt", "src", "src/profile.lua", "a.txt"].map(PathBuf::from)
    );
    assert_eq!(
        fs.read_directory("")
            .unwrap()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        ["z.txt", "src", "a.txt"].map(PathBuf::from)
    );
    assert_eq!(contents(&mut fs, "src/profile.lua"), "new");
    assert!(fs.read_directory("src/profile.lua").is_err());
    assert!(fs.absolute_path("src/profile.lua").unwrap().is_none());
    assert!(fs.read_file("unadmitted.lua").is_err());
}

#[test]
fn rejects_aliases_unsafe_components_and_incomplete_namespaces() {
    for path in [
        "/absolute",
        "../escape",
        "src/./x",
        "src//x",
        "x/",
        "C:/x",
        "a\\b",
        "con.txt",
        "COM1",
        "trailing.",
        "space ",
        "a:b",
        "é.lua",
    ] {
        assert!(
            View::new(
                "E:/fixture".into(),
                vec![dir(""), file(path, b"x")],
                budget(100, 10)
            )
            .is_err(),
            "{path}"
        );
    }
    for entries in [
        vec![dir(""), file("x", b"a"), file("X", b"b")],
        vec![dir(""), file("x", b"a"), file("x/y", b"b")],
        vec![dir(""), file("src/x", b"x")],
        vec![file("", b"x")],
        vec![dir("src")],
    ] {
        assert!(View::new("E:/fixture".into(), entries, budget(100, 10)).is_err());
    }
}

#[test]
fn recursive_effective_profile_is_read_for_inference_exactly_once() {
    let m = manifest("example");
    let e = extension();
    let lua = b"return {on_update=function()end}";
    let b = budget(100_000, 100);
    let mut fs = FileSystem::new_prepared(view(
        vec![
            dir(""),
            file("mod.json", &m),
            file("extended.mod.json", &e),
            dir("src"),
            file("src/profile.lua", lua),
        ],
        b.clone(),
    ));
    // No physical E:/prepared-fixture exists. This succeeds only through the
    // effective namespace; the ordinary ambient-profile route is refused.
    assert!(read_manifest(std::path::Path::new("E:/prepared-fixture"), &mut fs).is_err());
    let result = read_prepared_manifest(
        std::path::Path::new("E:/prepared-fixture"),
        &mut fs,
        policy(),
    )
    .unwrap();
    assert_eq!(result.settings[0].apply_at, "live");
    assert_eq!(
        b.counts().returned_bytes,
        (m.len() + e.len() + lua.len()) as u64
    );
    assert_eq!(
        contents(&mut fs, "src/profile.lua"),
        std::str::from_utf8(lua).unwrap()
    );
    assert_eq!(
        b.counts().returned_bytes,
        (m.len() + e.len() + 2 * lua.len()) as u64
    );
}

#[test]
fn strict_environment_loads_before_manifest_and_never_skips_bad_package() {
    let make = |id: &str| {
        view(
            vec![
                dir(""),
                file("mod.json", &manifest(id)),
                file("extended.mod.json", &extension()),
                dir("src"),
                file("src/init.lua", b"return {}"),
            ],
            budget(100_000, 100),
        )
    };
    let env = ModEnvironment::from_prepared_packages(
        "E:/prepared-fixture".into(),
        "E:/prepared-fixture/cache".into(),
        "E:/prepared-fixture/mods".into(),
        vec![make("example")],
        policy(),
    )
    .unwrap();
    assert!(env.is_mod_enabled("example"));
    assert_eq!(env.mod_registry().values().count(), 1);
    assert!(
        ModEnvironment::from_prepared_packages(
            "E:/prepared-fixture".into(),
            "E:/prepared-fixture/cache".into(),
            "E:/wrong/mods".into(),
            vec![make("example")],
            policy()
        )
        .is_err()
    );
    let bad = view(vec![dir(""), file("mod.json", b"{")], budget(100_000, 100));
    assert!(
        ModEnvironment::from_prepared_packages(
            "E:/prepared-fixture".into(),
            "E:/prepared-fixture/cache".into(),
            "E:/prepared-fixture/mods".into(),
            vec![make("example"), bad],
            policy()
        )
        .is_err()
    );
    assert!(
        ModEnvironment::from_prepared_packages(
            "E:/prepared-fixture".into(),
            "E:/prepared-fixture/cache".into(),
            "E:/prepared-fixture/mods".into(),
            vec![make("example"), make("example")],
            policy()
        )
        .is_err()
    );
}

#[test]
fn shared_budget_reserves_finalization_and_never_resets_per_reader() {
    let source = Source::bytes(Arc::from(&b"1234"[..]));
    let b = Budget::new(8, 2, 4, 1).unwrap();
    let mut first = source.reader(b.clone(), false);
    let mut raw = Vec::new();
    first.read_to_end(&mut raw).unwrap();
    assert_eq!(raw, b"1234");
    assert!(source.reader(b.clone(), false).read(&mut [0; 1]).is_err());
    assert_eq!((b.counts().charged_bytes, b.counts().calls), (4, 1));
    b.begin_finalization().unwrap();
    assert!(b.begin_finalization().is_err());
    assert!(source.reader(b.clone(), false).read(&mut [0; 1]).is_err());
    raw.clear();
    source
        .reader(b.clone(), true)
        .read_to_end(&mut raw)
        .unwrap();
    assert_eq!(
        (
            b.counts().charged_bytes,
            b.counts().returned_bytes,
            b.counts().calls
        ),
        (8, 8, 2)
    );
    assert!(Budget::new(3, 1, 4, 1).is_err());
    assert!(Budget::new(4, 0, 4, 1).is_err());
}

fn synthetic_path(name: &str) -> std::path::PathBuf {
    let root =
        std::path::PathBuf::from(r"E:\Build\sf178-prepared-view-synthetic-fixtures-20261104-r1");
    fs::create_dir_all(&root).unwrap();
    root.join(format!("{}-{name}", std::process::id()))
}

#[test]
fn held_file_readers_have_independent_positions_without_reopening_paths() {
    let path = synthetic_path("position.bin");
    fs::write(&path, b"123456").unwrap();
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let file = Arc::new(options.open(&path).unwrap());
    let source = Source::held(file, 6).unwrap();
    let b = budget(12, 10);
    let mut one = source.reader(b.clone(), false);
    let mut two = source.reader(b.clone(), false);
    let mut head = [0; 2];
    one.read_exact(&mut head).unwrap();
    assert_eq!(&head, b"12");
    let mut all = Vec::new();
    two.read_to_end(&mut all).unwrap();
    assert_eq!(all, b"123456");
    all.clear();
    one.read_to_end(&mut all).unwrap();
    assert_eq!(all, b"3456");
    assert_eq!(b.counts().returned_bytes, 12);
    #[cfg(windows)]
    assert!(OpenOptions::new().write(true).open(&path).is_err());
}

#[test]
fn short_held_read_records_actual_bytes_and_terminal_failure_without_retry() {
    let path = synthetic_path("short.bin");
    fs::write(&path, b"1234").unwrap();
    // Deliberately unprotected synthetic input tests the last-line short-read
    // refusal; this is not how the production admission lease opens files.
    let file = Arc::new(
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap(),
    );
    let source = Source::held(file.clone(), 4).unwrap();
    file.set_len(1).unwrap();
    let b = budget(100, 10);
    assert!(source.reader(b.clone(), false).read(&mut [0; 4]).is_err());
    assert_eq!(
        (
            b.counts().charged_bytes,
            b.counts().returned_bytes,
            b.counts().calls
        ),
        (4, 1, 1)
    );
    assert!(b.counts().terminal_io);
    assert!(b.begin_finalization().is_err());
    assert!(source.reader(b.clone(), false).read(&mut [0; 1]).is_err());
    assert_eq!(b.counts().calls, 1);
}

#[test]
fn zero_length_and_virtual_eof_do_not_probe_underlying_source() {
    let source = Source::bytes(Arc::from(&b""[..]));
    let b = budget(0, 0);
    assert_eq!(
        source.reader(b.clone(), false).read(&mut [0; 1]).unwrap(),
        0
    );
    assert_eq!((b.counts().charged_bytes, b.counts().calls), (0, 0));
    let mut fs = FileSystem::new_prepared(view(vec![dir("")], b));
    for path in ["../x", "/x", "C:/x"] {
        assert!(fs.read_file(path).is_err());
        assert!(!fs.exists(path));
    }
    assert!(fs.is_directory(Path::new("")));
}

#[test]
fn manifest_policy_is_explicit_and_invalid_optional_extension_is_not_hidden() {
    let mut value: serde_json::Value = serde_json::from_slice(&manifest("example")).unwrap();
    value["privateField"] = serde_json::json!("kept in raw source");
    let bytes = serde_json::to_vec(&value).unwrap();
    let make = || {
        FileSystem::new_prepared(view(
            vec![dir(""), file("mod.json", &bytes)],
            budget(10_000, 100),
        ))
    };
    let root = std::path::Path::new("E:/prepared-fixture");
    assert!(
        read_prepared_manifest(
            root,
            &mut make(),
            shroudforge_package::PreparedManifestPolicy {
                allow_unknown_manifest_fields: false
            }
        )
        .is_err()
    );
    assert!(read_prepared_manifest(root, &mut make(), policy()).is_ok());
    let mut bad = FileSystem::new_prepared(view(
        vec![
            dir(""),
            file("mod.json", &manifest("example")),
            file("extended.mod.json", b"{"),
        ],
        budget(10_000, 100),
    ));
    assert!(read_prepared_manifest(root, &mut bad, policy()).is_err());
}
