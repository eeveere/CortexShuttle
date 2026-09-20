use cortex_shuttle::{
    journal::{ToolCall, WorkspaceFileEdit, WorkspaceFilePatch, WorkspaceTextHunk},
    workspace::{
        MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES, MAX_WORKSPACE_TEXT_PATCH_FILES,
        MAX_WORKSPACE_TEXT_PATCH_HUNKS, MAX_WORKSPACE_TEXT_PATCH_HUNKS_PER_FILE,
        MAX_WORKSPACE_TEXT_PATCH_PATH_TOTAL_BYTES, MAX_WORKSPACE_TEXT_PATCH_TEXT_BYTES,
        MAX_WORKSPACE_TEXT_PATCH_TOTAL_FILE_BYTES, WorkspaceTextPatchBindings,
        WorkspaceTextPatchPreimage, plan_workspace_text_patch,
    },
};

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn bindings(paths: &[&str]) -> WorkspaceTextPatchBindings {
    WorkspaceTextPatchBindings {
        session_id: "session-1".into(),
        admission_id: "admission-1".into(),
        context_id: "context-1".into(),
        proposal_request_id: "request-1".into(),
        model_request_id: "model-request-1".into(),
        permission_id: "permission-1".into(),
        snapshot_id: "snapshot-1".into(),
        permitted_paths: paths.iter().map(|path| (*path).into()).collect(),
    }
}

fn preimage(path: &str, text: &str) -> WorkspaceTextPatchPreimage {
    WorkspaceTextPatchPreimage {
        path: path.into(),
        bytes: text.as_bytes().to_vec(),
    }
}

fn patch(path: &str, before: &str, hunks: &[(&str, &str)]) -> WorkspaceFilePatch {
    WorkspaceFilePatch {
        path: path.into(),
        expected_file_hash: hash(before.as_bytes()),
        hunks: hunks
            .iter()
            .map(|(old_utf8, new_utf8)| WorkspaceTextHunk {
                old_utf8: (*old_utf8).into(),
                new_utf8: (*new_utf8).into(),
            })
            .collect(),
    }
}

fn planned_text(plan: &cortex_shuttle::workspace::WorkspaceTextPatchPlan, path: &str) -> String {
    String::from_utf8(
        plan.postimages
            .iter()
            .find(|image| image.path == path)
            .unwrap()
            .bytes
            .clone(),
    )
    .unwrap()
}

#[test]
fn planner_builds_exact_replacement_deletion_and_crlf_anchor_insertion() {
    let cases = [
        (
            "replace.txt",
            "alpha\nbeta\n",
            vec![("beta", "gamma")],
            "alpha\ngamma\n",
        ),
        (
            "delete.txt",
            "alpha\nbeta\n",
            vec![("beta\n", "")],
            "alpha\n",
        ),
        (
            "insert.txt",
            "# Tests\r\nrun\r\n",
            vec![("# Tests\r\n", "# Tests\r\nOffline only.\r\n")],
            "# Tests\r\nOffline only.\r\nrun\r\n",
        ),
    ];
    for (path, before, hunks, after) in cases {
        let source = preimage(path, before);
        let plan = plan_workspace_text_patch(
            &bindings(&[path]),
            std::slice::from_ref(&source),
            &[patch(path, before, &hunks)],
        )
        .unwrap();
        assert_eq!(planned_text(&plan, path), after);
        assert_eq!(
            plan.patch.files[0].expected_postimage_hash,
            hash(after.as_bytes())
        );
        assert_eq!(source.bytes, before.as_bytes());
    }
}

#[test]
fn planner_resolves_unicode_on_byte_boundaries_and_canonicalizes_order() {
    let before = "éclair\nβeta\n";
    let forward = patch("src/file.txt", before, &[("βeta", "gamma"), ("é", "E")]);
    let reverse = patch("src/file.txt", before, &[("é", "E"), ("βeta", "gamma")]);
    let input = preimage("src/file.txt", before);
    let first = plan_workspace_text_patch(
        &bindings(&["src/file.txt"]),
        std::slice::from_ref(&input),
        &[forward],
    )
    .unwrap();
    let second =
        plan_workspace_text_patch(&bindings(&["src/file.txt"]), &[input], &[reverse]).unwrap();
    assert_eq!(planned_text(&first, "src/file.txt"), "Eclair\ngamma\n");
    assert_eq!(first.patch.patch_identity, second.patch.patch_identity);
    assert_eq!(first.patch.files[0].hunks[0].old_utf8, "é");
    assert_eq!(first.patch.files[0].hunks[0].start, 0);
}

#[test]
fn planner_allows_adjacent_hunks_and_binds_identity_to_session_permission() {
    let before = "abcdef";
    let source = preimage("file.txt", before);
    let file = patch("file.txt", before, &[("cd", "CD"), ("ab", "AB")]);
    let first = plan_workspace_text_patch(
        &bindings(&["file.txt"]),
        std::slice::from_ref(&source),
        std::slice::from_ref(&file),
    )
    .unwrap();
    let mut other_bindings = bindings(&["file.txt"]);
    other_bindings.permission_id = "permission-2".into();
    let second = plan_workspace_text_patch(&other_bindings, &[source], &[file]).unwrap();
    let mut third_bindings = bindings(&["file.txt"]);
    third_bindings.model_request_id = "model-request-2".into();
    let third = plan_workspace_text_patch(
        &third_bindings,
        &[preimage("file.txt", before)],
        &[patch("file.txt", before, &[("cd", "CD"), ("ab", "AB")])],
    )
    .unwrap();
    assert_eq!(planned_text(&first, "file.txt"), "ABCDef");
    assert_eq!(
        first.patch.files[0]
            .hunks
            .iter()
            .map(|hunk| (hunk.start, hunk.end))
            .collect::<Vec<_>>(),
        vec![(0, 2), (2, 4)]
    );
    assert_ne!(first.patch.patch_identity, second.patch.patch_identity);
    assert_ne!(first.patch.patch_identity, third.patch.patch_identity);
}

#[test]
fn planner_validates_all_files_before_returning_any_postimage() {
    let a = preimage("a.txt", "one\n");
    let b = preimage("b.txt", "two\n");
    let plan = plan_workspace_text_patch(
        &bindings(&["a.txt", "b.txt"]),
        &[a.clone(), b.clone()],
        &[
            patch("b.txt", "two\n", &[("two", "TWO")]),
            patch("a.txt", "one\n", &[("one", "ONE")]),
        ],
    )
    .unwrap();
    assert_eq!(
        plan.patch
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["a.txt", "b.txt"]
    );
    assert_eq!(planned_text(&plan, "a.txt"), "ONE\n");
    assert_eq!(planned_text(&plan, "b.txt"), "TWO\n");
    let reordered = plan_workspace_text_patch(
        &bindings(&["a.txt", "b.txt"]),
        &[a.clone(), b.clone()],
        &[
            patch("a.txt", "one\n", &[("one", "ONE")]),
            patch("b.txt", "two\n", &[("two", "TWO")]),
        ],
    )
    .unwrap();
    assert_eq!(plan.patch.patch_identity, reordered.patch.patch_identity);
    let rejected = plan_workspace_text_patch(
        &bindings(&["a.txt", "b.txt"]),
        &[a, b],
        &[
            patch("a.txt", "one\n", &[("one", "ONE")]),
            patch("b.txt", "two\n", &[("missing", "TWO")]),
        ],
    );
    assert!(rejected.is_err());
}

#[test]
fn planner_rejects_ambiguous_overlapping_invalid_and_noop_hunks() {
    let cases = [
        ("aaa", vec![("aa", "b")]),
        ("abc", vec![("ab", "x"), ("bc", "y")]),
        ("same", vec![("same", "same")]),
        ("anchor", vec![("", "insert")]),
        ("ab", vec![("a", "ab"), ("b", "")]),
        ("a", vec![("a", "b"), ("b", "c")]),
    ];
    for (before, hunks) in cases {
        assert!(
            plan_workspace_text_patch(
                &bindings(&["file.txt"]),
                &[preimage("file.txt", before)],
                &[patch("file.txt", before, &hunks)],
            )
            .is_err()
        );
    }
    let invalid_utf8 = WorkspaceTextPatchPreimage {
        path: "file.txt".into(),
        bytes: vec![0xff],
    };
    assert!(
        plan_workspace_text_patch(
            &bindings(&["file.txt"]),
            &[invalid_utf8],
            &[WorkspaceFilePatch {
                path: "file.txt".into(),
                expected_file_hash: hash(&[0xff]),
                hunks: vec![WorkspaceTextHunk {
                    old_utf8: "x".into(),
                    new_utf8: "y".into()
                }],
            }],
        )
        .is_err()
    );
}

#[test]
fn planner_rejects_wrong_grant_hash_duplicate_path_and_unsafe_path() {
    let before = "value\n";
    let source = preimage("file.txt", before);
    let mut wrong_hash = patch("file.txt", before, &[("value", "next")]);
    wrong_hash.expected_file_hash = "0".repeat(64);
    assert!(
        plan_workspace_text_patch(
            &bindings(&["file.txt"]),
            std::slice::from_ref(&source),
            &[wrong_hash]
        )
        .is_err()
    );
    assert!(
        plan_workspace_text_patch(
            &bindings(&["file.txt"]),
            std::slice::from_ref(&source),
            &[
                patch("file.txt", before, &[("value", "next")]),
                patch("file.txt", before, &[("value", "again")]),
            ],
        )
        .is_err()
    );
    assert!(
        plan_workspace_text_patch(
            &bindings(&["file.txt"]),
            &[source],
            &[patch("../file.txt", before, &[("value", "next")])],
        )
        .is_err()
    );
    let other = preimage("other.txt", before);
    assert!(
        plan_workspace_text_patch(
            &bindings(&["file.txt"]),
            &[preimage("file.txt", before), other],
            &[patch("other.txt", before, &[("value", "next")])],
        )
        .is_err()
    );
}

#[test]
fn planner_enforces_postimage_bound_without_side_effects() {
    let before = format!("a{}", "x".repeat(MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES - 1));
    let source = preimage("large.txt", &before);
    let result = plan_workspace_text_patch(
        &bindings(&["large.txt"]),
        std::slice::from_ref(&source),
        &[patch("large.txt", &before, &[("a", "aa")])],
    );
    assert!(result.is_err());
    assert_eq!(source.bytes.len(), MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES);
    assert_eq!(source.bytes, before.as_bytes());
}

#[test]
fn planner_enforces_inclusive_hunk_text_path_and_file_limits() {
    let before = (0..=MAX_WORKSPACE_TEXT_PATCH_HUNKS_PER_FILE)
        .map(|index| format!("anchor-{index}\n"))
        .collect::<String>();
    let hunks = (0..=MAX_WORKSPACE_TEXT_PATCH_HUNKS_PER_FILE)
        .map(|index| (format!("anchor-{index}"), format!("changed-{index}")))
        .collect::<Vec<_>>();
    let hunks = hunks
        .iter()
        .map(|(old, new)| (old.as_str(), new.as_str()))
        .collect::<Vec<_>>();
    assert!(
        plan_workspace_text_patch(
            &bindings(&["file.txt"]),
            &[preimage("file.txt", &before)],
            &[patch("file.txt", &before, &hunks)],
        )
        .is_err()
    );

    let too_many_total = (0..5)
        .map(|file| {
            let path = format!("many-{file}.txt");
            let before = (0..13)
                .map(|hunk| format!("old-{file}-{hunk:02}\n"))
                .collect::<String>();
            let hunks = (0..13)
                .map(|hunk| {
                    (
                        format!("old-{file}-{hunk:02}\n"),
                        format!("new-{file}-{hunk:02}\n"),
                    )
                })
                .collect::<Vec<_>>();
            let hunks = hunks
                .iter()
                .map(|(old, new)| (old.as_str(), new.as_str()))
                .collect::<Vec<_>>();
            (preimage(&path, &before), patch(&path, &before, &hunks))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        too_many_total.len() * 13,
        MAX_WORKSPACE_TEXT_PATCH_HUNKS + 1
    );
    assert!(
        plan_workspace_text_patch(
            &bindings(
                &too_many_total
                    .iter()
                    .map(|(file, _)| file.path.as_str())
                    .collect::<Vec<_>>()
            ),
            &too_many_total
                .iter()
                .map(|(file, _)| file.clone())
                .collect::<Vec<_>>(),
            &too_many_total
                .iter()
                .map(|(_, patch)| patch.clone())
                .collect::<Vec<_>>(),
        )
        .is_err()
    );

    let oversized_old = format!("a{}", "x".repeat(MAX_WORKSPACE_TEXT_PATCH_TEXT_BYTES));
    assert!(
        plan_workspace_text_patch(
            &bindings(&["file.txt"]),
            &[preimage("file.txt", &oversized_old)],
            &[patch("file.txt", &oversized_old, &[("a", "b")])],
        )
        .is_ok(),
        "the bound counts hunk text, not the complete preimage"
    );
    let old_hunk = "x".repeat(MAX_WORKSPACE_TEXT_PATCH_TEXT_BYTES);
    let text = format!("{old_hunk}!");
    assert!(
        plan_workspace_text_patch(
            &bindings(&["file.txt"]),
            &[preimage("file.txt", &text)],
            &[patch("file.txt", &text, &[(old_hunk.as_str(), "z")])],
        )
        .is_err()
    );

    let long_name = format!(
        "{}.txt",
        "a".repeat(MAX_WORKSPACE_TEXT_PATCH_PATH_TOTAL_BYTES - 4)
    );
    assert!(
        plan_workspace_text_patch(
            &bindings(&[long_name.as_str()]),
            &[preimage(&long_name, "x")],
            &[patch(&long_name, "x", &[("x", "y")])],
        )
        .is_err(),
        "a single path above 1 KiB fails before its aggregate bound"
    );

    let files = (0..=MAX_WORKSPACE_TEXT_PATCH_FILES)
        .map(|index| {
            let path = format!("{index}.txt");
            (preimage(&path, "x"), patch(&path, "x", &[("x", "y")]))
        })
        .collect::<Vec<_>>();
    assert!(
        plan_workspace_text_patch(
            &bindings(
                &files
                    .iter()
                    .map(|(file, _)| file.path.as_str())
                    .collect::<Vec<_>>()
            ),
            &files
                .iter()
                .map(|(file, _)| file.clone())
                .collect::<Vec<_>>(),
            &files
                .iter()
                .map(|(_, patch)| patch.clone())
                .collect::<Vec<_>>(),
        )
        .is_err()
    );
}

#[test]
fn planner_accepts_exact_hunk_text_path_file_and_image_aggregate_limits() {
    let per_file_before = (0..MAX_WORKSPACE_TEXT_PATCH_HUNKS_PER_FILE)
        .map(|index| format!("old-{index:02}\n"))
        .collect::<String>();
    let per_file_hunks = (0..MAX_WORKSPACE_TEXT_PATCH_HUNKS_PER_FILE)
        .map(|index| (format!("old-{index:02}\n"), format!("new-{index:02}\n")))
        .collect::<Vec<_>>();
    let per_file_hunks = per_file_hunks
        .iter()
        .map(|(old, new)| (old.as_str(), new.as_str()))
        .collect::<Vec<_>>();
    assert!(
        plan_workspace_text_patch(
            &bindings(&["per-file.txt"]),
            &[preimage("per-file.txt", &per_file_before)],
            &[patch("per-file.txt", &per_file_before, &per_file_hunks)],
        )
        .is_ok()
    );

    let exact_file_count = (0..MAX_WORKSPACE_TEXT_PATCH_FILES)
        .map(|index| {
            let path = format!("exact-{index}.txt");
            (preimage(&path, "x"), patch(&path, "x", &[("x", "y")]))
        })
        .collect::<Vec<_>>();
    assert!(
        plan_workspace_text_patch(
            &bindings(
                &exact_file_count
                    .iter()
                    .map(|(file, _)| file.path.as_str())
                    .collect::<Vec<_>>()
            ),
            &exact_file_count
                .iter()
                .map(|(file, _)| file.clone())
                .collect::<Vec<_>>(),
            &exact_file_count
                .iter()
                .map(|(_, patch)| patch.clone())
                .collect::<Vec<_>>(),
        )
        .is_ok()
    );

    let aggregate = (0..4)
        .flat_map(|file| {
            (0..MAX_WORKSPACE_TEXT_PATCH_HUNKS_PER_FILE).map(move |hunk| {
                let path = format!("aggregate-{file}.txt");
                (
                    path,
                    format!("old-{file}-{hunk:02}\n"),
                    format!("new-{file}-{hunk:02}\n"),
                )
            })
        })
        .collect::<Vec<_>>();
    let files = (0..4)
        .map(|file| {
            let path = format!("aggregate-{file}.txt");
            let before = aggregate
                .iter()
                .filter(|(candidate, _, _)| candidate == &path)
                .map(|(_, old, _)| old.clone())
                .collect::<String>();
            let hunks = aggregate
                .iter()
                .filter(|(candidate, _, _)| candidate == &path)
                .map(|(_, old, new)| (old.as_str(), new.as_str()))
                .collect::<Vec<_>>();
            (preimage(&path, &before), patch(&path, &before, &hunks))
        })
        .collect::<Vec<_>>();
    assert_eq!(aggregate.len(), MAX_WORKSPACE_TEXT_PATCH_HUNKS);
    assert!(
        plan_workspace_text_patch(
            &bindings(
                &files
                    .iter()
                    .map(|(file, _)| file.path.as_str())
                    .collect::<Vec<_>>()
            ),
            &files
                .iter()
                .map(|(file, _)| file.clone())
                .collect::<Vec<_>>(),
            &files
                .iter()
                .map(|(_, patch)| patch.clone())
                .collect::<Vec<_>>(),
        )
        .is_ok()
    );

    let old = "a".repeat(MAX_WORKSPACE_TEXT_PATCH_TEXT_BYTES / 2);
    let new = "b".repeat(MAX_WORKSPACE_TEXT_PATCH_TEXT_BYTES / 2);
    let before = format!("{old}!");
    assert!(
        plan_workspace_text_patch(
            &bindings(&["text.txt"]),
            &[preimage("text.txt", &before)],
            &[patch("text.txt", &before, &[(old.as_str(), new.as_str())])],
        )
        .is_ok()
    );

    let first_path = format!("{}.txt", "a".repeat(1_020));
    let second_path = format!("{}.txt", "b".repeat(1_020));
    assert_eq!(
        first_path.len() + second_path.len(),
        MAX_WORKSPACE_TEXT_PATCH_PATH_TOTAL_BYTES
    );
    assert!(
        plan_workspace_text_patch(
            &bindings(&[first_path.as_str(), second_path.as_str()]),
            &[preimage(&first_path, "x"), preimage(&second_path, "x")],
            &[
                patch(&first_path, "x", &[("x", "y")]),
                patch(&second_path, "x", &[("x", "y")]),
            ],
        )
        .is_ok()
    );

    let image_files = (0..4)
        .map(|index| {
            let path = format!("image-{index}.txt");
            let bytes = format!("a{}", "x".repeat(MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES - 1));
            (preimage(&path, &bytes), patch(&path, &bytes, &[("a", "b")]))
        })
        .collect::<Vec<_>>();
    assert!(
        plan_workspace_text_patch(
            &bindings(
                &image_files
                    .iter()
                    .map(|(file, _)| file.path.as_str())
                    .collect::<Vec<_>>()
            ),
            &image_files
                .iter()
                .map(|(file, _)| file.clone())
                .collect::<Vec<_>>(),
            &image_files
                .iter()
                .map(|(_, patch)| patch.clone())
                .collect::<Vec<_>>(),
        )
        .is_ok()
    );
}

#[test]
fn planner_rejects_overflowing_aggregate_image_and_path_limits() {
    let image_files = (0..5)
        .map(|index| {
            let path = format!("image-{index}.txt");
            let bytes = format!("a{}", "x".repeat(MAX_WORKSPACE_TEXT_PATCH_FILE_BYTES - 1));
            (preimage(&path, &bytes), patch(&path, &bytes, &[("a", "b")]))
        })
        .collect::<Vec<_>>();
    assert!(
        plan_workspace_text_patch(
            &bindings(
                &image_files
                    .iter()
                    .map(|(file, _)| file.path.as_str())
                    .collect::<Vec<_>>()
            ),
            &image_files
                .iter()
                .map(|(file, _)| file.clone())
                .collect::<Vec<_>>(),
            &image_files
                .iter()
                .map(|(_, patch)| patch.clone())
                .collect::<Vec<_>>(),
        )
        .is_err()
    );
    let per_file_pre = MAX_WORKSPACE_TEXT_PATCH_TOTAL_FILE_BYTES / 5;
    let postimage_only_overflow = (0..5)
        .map(|index| {
            let path = format!("post-{index}.txt");
            let text = format!("a{}", "x".repeat(per_file_pre - 1));
            (preimage(&path, &text), patch(&path, &text, &[("a", "aa")]))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        per_file_pre * 5,
        MAX_WORKSPACE_TEXT_PATCH_TOTAL_FILE_BYTES - 4
    );
    assert!(
        plan_workspace_text_patch(
            &bindings(
                &postimage_only_overflow
                    .iter()
                    .map(|(file, _)| file.path.as_str())
                    .collect::<Vec<_>>()
            ),
            &postimage_only_overflow
                .iter()
                .map(|(file, _)| file.clone())
                .collect::<Vec<_>>(),
            &postimage_only_overflow
                .iter()
                .map(|(_, patch)| patch.clone())
                .collect::<Vec<_>>(),
        )
        .is_err()
    );
    let names = ["a".repeat(683), "b".repeat(683), "c".repeat(683)];
    assert!(
        plan_workspace_text_patch(
            &bindings(&names.iter().map(String::as_str).collect::<Vec<_>>()),
            &names
                .iter()
                .map(|path| preimage(path, "x"))
                .collect::<Vec<_>>(),
            &names
                .iter()
                .map(|path| patch(path, "x", &[("x", "y")]))
                .collect::<Vec<_>>(),
        )
        .is_err()
    );
}

#[test]
fn planner_enforces_serialized_prepared_patch_limit_with_escaped_content() {
    let old = "\u{0001}".repeat(MAX_WORKSPACE_TEXT_PATCH_TEXT_BYTES / 2);
    let new = "\u{0002}".repeat(MAX_WORKSPACE_TEXT_PATCH_TEXT_BYTES / 2);
    let before = format!("{old}!");
    let mut escaped = bindings(&["file.txt"]);
    let component = "\u{0003}".repeat(1_024);
    escaped.session_id = component.clone();
    escaped.admission_id = component.clone();
    escaped.context_id = component.clone();
    escaped.proposal_request_id = component.clone();
    escaped.model_request_id = component.clone();
    escaped.permission_id = component.clone();
    escaped.snapshot_id = component;
    assert!(
        plan_workspace_text_patch(
            &escaped,
            &[preimage("file.txt", &before)],
            &[patch("file.txt", &before, &[(old.as_str(), new.as_str())])],
        )
        .is_err()
    );
}

#[test]
fn legacy_whole_file_actions_still_decode_after_v2_types_are_added() {
    let legacy = ToolCall::WriteWorkspaceFiles {
        edits: vec![WorkspaceFileEdit {
            path: "file.txt".into(),
            expected_hash: "a".repeat(64),
            utf8_bytes: b"legacy\n".to_vec(),
        }],
    };
    let encoded = serde_json::to_vec(&legacy).unwrap();
    assert_eq!(
        serde_json::from_slice::<ToolCall>(&encoded).unwrap(),
        legacy
    );
    let prepared = plan_workspace_text_patch(
        &bindings(&["file.txt"]),
        &[preimage("file.txt", "before")],
        &[patch("file.txt", "before", &[("before", "after")])],
    )
    .unwrap()
    .patch;
    let v2 = ToolCall::PatchWorkspaceFiles {
        patch: prepared.clone(),
    };
    assert_eq!(
        serde_json::from_slice::<ToolCall>(&serde_json::to_vec(&v2).unwrap()).unwrap(),
        v2
    );
    assert!(
        serde_json::from_slice::<cortex_shuttle::journal::WorkspaceTextPatch>(&encoded).is_err()
    );
}
