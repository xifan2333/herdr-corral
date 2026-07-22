use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn diff_reuses_owned_nvim_via_rpc_without_terminal_input_injection() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = std::env::temp_dir().join(format!(
        "corral-preview-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let uid = String::from_utf8(Command::new("id").arg("-u").output().unwrap().stdout).unwrap();
    let runtime = temp.join(format!("corral-{}", uid.trim()));
    fs::create_dir_all(&runtime).unwrap();
    let socket = runtime.join("nvim-w_p1.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    let herdr = temp.join("herdr");
    let nvim = temp.join("nvim");
    let corral = temp.join("corral");
    let herdr_log = temp.join("herdr.log");
    let nvim_log = temp.join("nvim.log");

    fs::write(
        &herdr,
        r#"#!/usr/bin/env bash
printf '%q ' "$@" >>"$FAKE_HERDR_LOG"; printf '\n' >>"$FAKE_HERDR_LOG"
case "$1 $2" in
  'pane list') printf '%s\n' '{"result":{"panes":[{"pane_id":"editor","tokens":{"corral-editor-owner":"w:p1"}}]}}' ;;
  'pane process-info') printf '%s\n' '{"result":{"process_info":{"foreground_processes":[{"name":"nvim"}]}}}' ;;
esac
"#,
    )
    .unwrap();
    fs::write(
        &nvim,
        r#"#!/usr/bin/env bash
printf '%q ' "$@" >>"$FAKE_NVIM_LOG"; printf '\n' >>"$FAKE_NVIM_LOG"
[[ " $* " == *" --remote-expr 1 "* ]] && printf '1\n'
exit 0
"#,
    )
    .unwrap();
    fs::write(&corral, "#!/usr/bin/env bash\nexit 0\n").unwrap();
    for path in [&herdr, &nvim, &corral] {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    let path = format!("{}:/usr/bin:/bin", temp.display());
    let status = Command::new("bash")
        .arg("-c")
        .arg("source \"$1\"; _corral_run 'printf preview-ok'")
        .arg("--")
        .arg(root.join("config.default.sh"))
        .env("PATH", path)
        .env("EDITOR", "nvim")
        .env("XDG_RUNTIME_DIR", &temp)
        .env("HERDR_BIN_PATH", &herdr)
        .env("HERDR_ENV", "1")
        .env("HERDR_PANE_ID", "w:p1")
        .env("FAKE_HERDR_LOG", &herdr_log)
        .env("FAKE_NVIM_LOG", &nvim_log)
        .status()
        .unwrap();
    assert!(status.success());

    let herdr_calls = fs::read_to_string(&herdr_log).unwrap();
    let nvim_calls = fs::read_to_string(&nvim_log).unwrap();
    assert!(herdr_calls.contains("pane process-info --pane editor"));
    assert!(!herdr_calls.contains("send-text"));
    assert!(!herdr_calls.contains("send-keys"));
    assert!(!herdr_calls.contains("pane split"));
    assert!(!herdr_calls.contains("pane run"));
    assert!(nvim_calls.contains("--server"));
    assert!(nvim_calls.contains("--remote-expr"));
    assert!(nvim_calls.contains("nonumber"));
    assert!(nvim_calls.contains("norelativenumber"));
    assert!(nvim_calls.contains("signcolumn=no"));
    assert!(nvim_calls.contains("foldcolumn=0"));
    assert!(nvim_calls.contains("terminal"));

    fs::remove_dir_all(temp).unwrap();
}
