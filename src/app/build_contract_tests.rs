#[test]
fn windows_resource_build_embeds_manifest_as_resource() {
    let build_rs = include_str!("../../build.rs");
    let cargo_toml = include_str!("../../Cargo.toml");
    let manifest = include_str!("../../app.manifest");

    assert!(
        build_rs.contains("set_manifest_file(\"app.manifest\")"),
        "build.rs must embed app.manifest as an RT_MANIFEST resource via winres for release builds"
    );
    assert!(
        build_rs.contains(r#"Ok("release")"#),
        "the admin manifest must stay scoped to release builds so cargo test remains runnable"
    );
    assert!(
        build_rs.contains("cargo:rerun-if-changed=app.manifest"),
        "manifest edits must trigger resource rebuilds"
    );
    assert!(
        invalid_winres_manifest_metadata_keys(cargo_toml).is_empty(),
        "app.manifest must be embedded by build.rs, not by package.metadata.winres manifest keys"
    );
    assert!(
        manifest
            .contains(r#"requestedExecutionLevel level="requireAdministrator" uiAccess="false""#),
        "app.manifest must retain the documented elevation and uiAccess contract"
    );
}

#[test]
fn winres_metadata_allows_non_manifest_keys() {
    let cargo_toml = r#"
[package.metadata.winres]
FileDescription = "CPU Affinity Tool"
LegalCopyright = "MIT"

[dependencies]
manifest = "not a winres metadata key"
"#;

    assert!(invalid_winres_manifest_metadata_keys(cargo_toml).is_empty());
}

#[test]
fn winres_metadata_rejects_manifest_embedding_keys() {
    let cargo_toml = r#"
[package.metadata.winres]
manifest = "app.manifest"
manifest_file = "app.manifest"

[package.metadata.other]
manifest = "unrelated"
"#;

    assert_eq!(
        invalid_winres_manifest_metadata_keys(cargo_toml),
        vec!["manifest".to_string(), "manifest_file".to_string()]
    );
}

fn invalid_winres_manifest_metadata_keys(cargo_toml: &str) -> Vec<String> {
    let mut in_winres_metadata = false;
    let mut invalid_keys = Vec::new();

    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_winres_metadata = trimmed == "[package.metadata.winres]";
            continue;
        }

        if !in_winres_metadata {
            continue;
        }

        let Some((raw_key, _)) = trimmed.split_once('=') else {
            continue;
        };
        let key = raw_key.trim().trim_matches('"').trim_matches('\'');
        if matches!(key, "manifest" | "manifest_file") {
            invalid_keys.push(key.to_string());
        }
    }

    invalid_keys
}
