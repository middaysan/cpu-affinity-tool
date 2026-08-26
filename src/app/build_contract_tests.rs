use std::fs;
use std::path::{Path, PathBuf};

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
fn binaries_use_the_platform_system_allocator() {
    let cargo_toml = include_str!("../../Cargo.toml");
    let cargo_lock = include_str!("../../Cargo.lock");

    assert!(
        !toml_has_dependency(cargo_toml, "mimalloc"),
        "the application manifest must not depend on mimalloc"
    );
    assert!(
        !cargo_lock_has_package(cargo_lock, "mimalloc")
            && !cargo_lock_has_package(cargo_lock, "libmimalloc-sys"),
        "the resolved dependency graph must not retain mimalloc"
    );

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_roots = [repo_root.join("src"), repo_root.join("libs")];
    let offenders = rust_sources_with_global_allocator(repo_root, &source_roots);
    assert!(
        offenders.is_empty(),
        "all binaries must use the platform system allocator; global allocator declarations found in: {offenders:?}"
    );
}

#[test]
fn windows_ci_and_stable_release_share_the_symbol_build_contract() {
    let cargo_toml = include_str!("../../Cargo.toml");
    let ci_workflow = include_str!("../../.github/workflows/ci.yml");
    let release_workflow = include_str!("../../.github/workflows/release.yml");
    let linux_release_workflow = include_str!("../../.github/workflows/release-linux-beta.yml");
    let release_build_script = include_str!("../../scripts/build-windows-release.ps1");
    let pdb_verifier = include_str!("../../scripts/assert-windows-pdb-matches.ps1");
    let pdb_verifier_tests = include_str!("../../scripts/test-windows-pdb-verifier.ps1");

    let ci_build = yaml_named_step(ci_workflow, "Build Windows release binary")
        .expect("Windows CI must have a named release build step");
    let release_build = yaml_named_step(release_workflow, "Build Windows release binary")
        .expect("the stable workflow must have a named release build step");
    let ci_verify = yaml_named_step(ci_workflow, "Verify Windows debug symbols")
        .expect("Windows CI must verify its release PDB");
    let release_verify = yaml_named_step(release_workflow, "Verify Windows debug symbols")
        .expect("the stable workflow must verify its release PDB");
    let release_upload = yaml_named_step(release_workflow, "Upload Windows release artifacts")
        .expect("the stable workflow must upload named Windows artifacts");
    let github_release = yaml_named_step(release_workflow, "Create Release")
        .expect("the stable workflow must have a named GitHub Release step");

    assert!(
        !toml_table_has_exact_key(cargo_toml, "profile.release", "debug"),
        "Windows line tables must not change the shared release profile"
    );
    assert!(
        yaml_step_has_active_text(&ci_build, "run: ./scripts/build-windows-release.ps1")
            && yaml_step_has_active_text(
                &release_build,
                "run: ./scripts/build-windows-release.ps1",
            ),
        "Windows CI and stable release must use the same line-table release build helper"
    );
    assert!(
        release_build_script.contains("line-tables-only")
            && release_build_script
                .contains("cargo build --release --features windows --bin cpu-affinity-tool"),
        "the shared Windows release helper must build the production binary with line tables"
    );
    assert!(
        yaml_step_has_active_text(&ci_verify, "./scripts/test-windows-pdb-verifier.ps1")
            && yaml_step_has_active_text(
                &release_verify,
                "./scripts/assert-windows-pdb-matches.ps1",
            )
            && yaml_step_has_active_text(&ci_verify, "target/release/cpu-affinity-tool.exe")
            && yaml_step_has_active_text(&ci_verify, "target/release/cpu_affinity_tool.pdb")
            && yaml_step_has_active_text(&release_verify, "target/release/cpu-affinity-tool.exe")
            && yaml_step_has_active_text(&release_verify, "target/release/cpu_affinity_tool.pdb"),
        "Windows CI must test the PDB verifier and stable release must compare EXE/PDB identity"
    );
    assert!(
        pdb_verifier.contains("SymSrvGetFileIndexInfoW")
            && pdb_verifier.contains("$exeIndex.guid -ne $pdbIndex.guid")
            && pdb_verifier.contains("$exeIndex.age -ne $pdbIndex.age")
            && pdb_verifier_tests.contains("*Missing Windows PDB*")
            && pdb_verifier_tests.contains("*Windows PDB is empty*")
            && pdb_verifier_tests.contains("*basename mismatch*")
            && pdb_verifier_tests.contains("*identity mismatch*"),
        "the dependency-free verifier must compare basename, GUID, and age with negative coverage"
    );
    assert!(
        yaml_step_has_active_text(&release_upload, "target/release/cpu-affinity-tool.exe")
            && yaml_step_has_active_text(&release_upload, "target/release/cpu_affinity_tool.pdb")
            && yaml_step_has_active_text(&release_upload, "if-no-files-found: error"),
        "the upload step must declare both Windows files and reject a fully unmatched path set; the preceding identity verifier requires each file individually"
    );
    assert!(
        yaml_step_has_active_text(&github_release, "./artifacts/windows/cpu-affinity-tool.exe")
            && yaml_step_has_active_text(
                &github_release,
                "./artifacts/windows/cpu_affinity_tool.pdb",
            )
            && yaml_step_has_active_text(&github_release, "fail_on_unmatched_files: true"),
        "the GitHub Release step must reject missing declared artifacts"
    );
    assert!(
        !yaml_step_has_active_text(linux_release_workflow, "build-windows-release.ps1")
            && !yaml_step_has_active_text(linux_release_workflow, "CARGO_PROFILE_RELEASE_DEBUG"),
        "the Windows symbol build must not change Linux beta debug policy"
    );
}

#[test]
fn contract_parsers_scope_exact_keys_and_named_steps() {
    let cargo_toml = r#"
[profile.release]
debug-assertions = false
strip = "symbols"

[profile.dev]
debug = true
"#;
    assert!(!toml_table_has_exact_key(
        cargo_toml,
        "profile.release",
        "debug"
    ));
    assert!(toml_table_has_exact_key(
        "[profile.release]\ndebug = \"line-tables-only\"",
        "profile.release",
        "debug"
    ));
    assert!(toml_table_has_exact_key(
        "[profile.release] # production profile\ndebug = \"line-tables-only\" # symbols",
        "profile.release",
        "debug"
    ));
    assert!(!toml_table_has_exact_key(
        "[profile.release]\nstrip = \"symbols\"\n[profile.dev] # another table\ndebug = true",
        "profile.release",
        "debug"
    ));

    let workflow = r#"
steps:
  - name: Unrelated step
    run: echo CARGO_PROFILE_RELEASE_DEBUG
  - name: Build Windows release binary
    run: ./scripts/build-windows-release.ps1
next-job:
  steps:
    - name: Later step
      run: echo target/release/cpu_affinity_tool.pdb
"#;
    let build_step = yaml_named_step(workflow, "Build Windows release binary").unwrap();
    assert!(yaml_step_has_active_text(
        &build_step,
        "build-windows-release.ps1"
    ));
    assert!(!yaml_step_has_active_text(
        &build_step,
        "CARGO_PROFILE_RELEASE_DEBUG"
    ));
    assert!(!yaml_step_has_active_text(
        &build_step,
        "cpu_affinity_tool.pdb"
    ));

    let commented_workflow = r#"
steps:
  - name: Build Windows release binary
    # run: ./scripts/build-windows-release.ps1
    run: echo skipped
"#;
    let commented_step =
        yaml_named_step(commented_workflow, "Build Windows release binary").unwrap();
    assert!(!yaml_step_has_active_text(
        &commented_step,
        "./scripts/build-windows-release.ps1"
    ));

    let inert_rust = "// #[global_allocator]\n/* outer /* #[global_allocator] */ comment */\nconst COOKED: &str = \"#[global_allocator]\";\nconst RAW: &str = r#\"#[global_allocator]\"#;\nconst RAW_BYTES: &[u8] = br#\"#[global_allocator]\"#;";
    assert!(!rust_source_has_global_allocator(inert_rust));
    assert!(rust_source_has_global_allocator(
        "# /* an attribute comment */ [ global_allocator ]\nstatic ALLOCATOR: System = System;"
    ));
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
        let trimmed = strip_unquoted_hash_comment(line).trim();
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

fn toml_table_has_exact_key(toml: &str, table: &str, expected_key: &str) -> bool {
    let expected_header = format!("[{table}]");
    let mut in_table = false;

    for line in toml.lines() {
        let trimmed = strip_unquoted_hash_comment(line).trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_table = trimmed == expected_header;
            continue;
        }
        if !in_table {
            continue;
        }

        let Some((raw_key, _)) = trimmed.split_once('=') else {
            continue;
        };
        let key = raw_key.trim().trim_matches('"').trim_matches('\'');
        if key == expected_key {
            return true;
        }
    }

    false
}

fn toml_has_dependency(toml: &str, expected_key: &str) -> bool {
    let mut in_dependencies = false;

    for line in toml.lines() {
        let trimmed = strip_unquoted_hash_comment(line).trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_dependencies = trimmed == "[dependencies]" || trimmed.ends_with(".dependencies]");
            continue;
        }
        if !in_dependencies {
            continue;
        }

        let Some((raw_key, _)) = trimmed.split_once('=') else {
            continue;
        };
        let key = raw_key.trim().trim_matches('"').trim_matches('\'');
        if key == expected_key {
            return true;
        }
    }

    false
}

fn cargo_lock_has_package(cargo_lock: &str, expected_name: &str) -> bool {
    let mut in_package = false;

    for line in cargo_lock.lines() {
        let trimmed = strip_unquoted_hash_comment(line).trim();
        if trimmed == "[[package]]" {
            in_package = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_package = false;
            continue;
        }
        if !in_package {
            continue;
        }

        let Some((raw_key, raw_value)) = trimmed.split_once('=') else {
            continue;
        };
        if raw_key.trim() == "name" && raw_value.trim().trim_matches('"') == expected_name {
            return true;
        }
    }

    false
}

fn rust_sources_with_global_allocator(repo_root: &Path, source_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut pending = source_roots.to_vec();
    let mut offenders = Vec::new();

    while let Some(path) = pending.pop() {
        let entries = fs::read_dir(&path)
            .unwrap_or_else(|error| panic!("failed to inspect {}: {error}", path.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|error| {
                panic!(
                    "failed to inspect an entry under {}: {error}",
                    path.display()
                )
            });
            let child = entry.path();
            let file_type = entry.file_type().unwrap_or_else(|error| {
                panic!(
                    "failed to inspect file type for {}: {error}",
                    child.display()
                )
            });
            if file_type.is_dir() {
                let name = child.file_name().and_then(|value| value.to_str());
                if matches!(name, Some(".git" | "target" | "tasks")) {
                    continue;
                }
                pending.push(child);
                continue;
            }
            if child.extension().and_then(|value| value.to_str()) != Some("rs") {
                continue;
            }

            let source = fs::read_to_string(&child)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", child.display()));
            if rust_source_has_global_allocator(&source) {
                offenders.push(
                    child
                        .strip_prefix(repo_root)
                        .unwrap_or(&child)
                        .to_path_buf(),
                );
            }
        }
    }

    offenders.sort();
    offenders
}

fn yaml_named_step(workflow: &str, step_name: &str) -> Option<String> {
    let marker = format!("- name: {step_name}");
    let lines = workflow.lines().collect::<Vec<_>>();
    let start = lines.iter().position(|line| line.trim() == marker)?;
    let indentation = lines[start].len() - lines[start].trim_start().len();
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, line)| {
            let trimmed = line.trim_start();
            let current_indentation = line.len() - trimmed.len();
            (!trimmed.is_empty()
                && (current_indentation < indentation
                    || (current_indentation == indentation && trimmed.starts_with("- "))))
            .then_some(index)
        })
        .unwrap_or(lines.len());

    Some(lines[start..end].join("\n"))
}

fn yaml_step_has_active_text(yaml: &str, expected: &str) -> bool {
    yaml.lines().any(|line| {
        let active = strip_unquoted_hash_comment(line).trim();
        !active.is_empty() && active.contains(expected)
    })
}

fn strip_unquoted_hash_comment(line: &str) -> &str {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;

    for (index, character) in line.char_indices() {
        if double_quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                double_quoted = false;
            }
            continue;
        }
        if single_quoted {
            if character == '\'' {
                single_quoted = false;
            }
            continue;
        }

        match character {
            '\'' => single_quoted = true,
            '"' => double_quoted = true,
            '#' => return &line[..index],
            _ => {}
        }
    }

    line
}

fn rust_source_has_global_allocator(source: &str) -> bool {
    rust_source_has_attribute(source, "#[global_allocator]")
}

fn rust_source_has_attribute(source: &str, compact_attribute: &str) -> bool {
    let code = rust_code_without_comments_and_literals(source.as_bytes());
    let compact_code = code
        .into_iter()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    compact_code
        .windows(compact_attribute.len())
        .any(|window| window == compact_attribute.as_bytes())
}

fn rust_code_without_comments_and_literals(source: &[u8]) -> Vec<u8> {
    let mut code = Vec::with_capacity(source.len());
    let mut index = 0usize;

    while index < source.len() {
        if source[index..].starts_with(b"//") {
            index += 2;
            while index < source.len() && source[index] != b'\n' {
                index += 1;
            }
            code.push(b' ');
            continue;
        }
        if source[index..].starts_with(b"/*") {
            index = rust_block_comment_end(source, index + 2);
            code.push(b' ');
            continue;
        }
        if let Some(end) = rust_raw_string_end(source, index) {
            index = end;
            code.push(b' ');
            continue;
        }
        if let Some(quote_index) = rust_cooked_string_quote(source, index) {
            index = rust_quoted_literal_end(source, quote_index, b'"');
            code.push(b' ');
            continue;
        }
        if let Some(quote_index) = rust_char_literal_quote(source, index) {
            if let Some(end) = rust_char_literal_end(source, quote_index) {
                index = end;
                code.push(b' ');
                continue;
            }
        }

        code.push(source[index]);
        index += 1;
    }

    code
}

fn rust_block_comment_end(source: &[u8], mut index: usize) -> usize {
    let mut depth = 1usize;
    while index < source.len() && depth > 0 {
        if source[index..].starts_with(b"/*") {
            depth += 1;
            index += 2;
        } else if source[index..].starts_with(b"*/") {
            depth -= 1;
            index += 2;
        } else {
            index += 1;
        }
    }
    index
}

fn rust_raw_string_end(source: &[u8], start: usize) -> Option<usize> {
    let raw_prefix = if source.get(start) == Some(&b'r') {
        start
    } else if matches!(source.get(start), Some(b'b' | b'c')) && source.get(start + 1) == Some(&b'r')
    {
        start + 1
    } else {
        return None;
    };

    let mut opening_quote = raw_prefix + 1;
    while source.get(opening_quote) == Some(&b'#') {
        opening_quote += 1;
    }
    if source.get(opening_quote) != Some(&b'"') {
        return None;
    }

    let hash_count = opening_quote - raw_prefix - 1;
    let mut cursor = opening_quote + 1;
    while cursor < source.len() {
        if source[cursor] == b'"'
            && source
                .get(cursor + 1..cursor + 1 + hash_count)
                .is_some_and(|hashes| hashes.iter().all(|byte| *byte == b'#'))
        {
            return Some(cursor + 1 + hash_count);
        }
        cursor += 1;
    }

    Some(source.len())
}

fn rust_cooked_string_quote(source: &[u8], start: usize) -> Option<usize> {
    if source.get(start) == Some(&b'"') {
        Some(start)
    } else if matches!(source.get(start), Some(b'b' | b'c')) && source.get(start + 1) == Some(&b'"')
    {
        Some(start + 1)
    } else {
        None
    }
}

fn rust_char_literal_quote(source: &[u8], start: usize) -> Option<usize> {
    if source.get(start) == Some(&b'\'') {
        Some(start)
    } else if source.get(start) == Some(&b'b') && source.get(start + 1) == Some(&b'\'') {
        Some(start + 1)
    } else {
        None
    }
}

fn rust_char_literal_end(source: &[u8], quote_index: usize) -> Option<usize> {
    let content_start = quote_index + 1;
    let first = *source.get(content_start)?;
    if first == b'\\' {
        let end = rust_quoted_literal_end(source, quote_index, b'\'');
        return (end > content_start + 1).then_some(end);
    }

    let remaining = std::str::from_utf8(source.get(content_start..)?).ok()?;
    let character = remaining.chars().next()?;
    let closing_quote = content_start + character.len_utf8();
    (source.get(closing_quote) == Some(&b'\'')).then_some(closing_quote + 1)
}

fn rust_quoted_literal_end(source: &[u8], quote_index: usize, quote: u8) -> usize {
    let mut index = quote_index + 1;
    let mut escaped = false;
    while index < source.len() {
        let byte = source[index];
        index += 1;
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == quote {
            return index;
        } else if byte == b'\n' && quote == b'\'' {
            return quote_index + 1;
        }
    }
    source.len()
}
