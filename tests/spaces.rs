//! End-to-end check that an executable path/argument containing spaces is
//! passed through to the launched process as a single argument.
//!
//! This is the regression test for the original `game.sh` bug where the
//! pressure-vessel string round-trip re-split such paths on whitespace.

use std::process::Command;

#[test]
fn space_path_passed_as_single_argument() {
    let bin = env!("CARGO_BIN_EXE_game");
    let space_arg = "/tmp/with spaces/Orebits Demo V1.0";

    // Wrappers disabled (-g -h -k), silent mode (-l -1) so the command is run
    // directly. `printf '%s\n'` prints each argument on its own line, so a
    // split argument would yield multiple lines.
    let output = Command::new(bin)
        .args(["-g", "-h", "-k", "-l", "-1", "--", "printf", "%s\\n", space_arg])
        .env("HOME", std::env::temp_dir())
        .output()
        .expect("failed to run game binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();

    assert_eq!(lines.len(), 1, "argument was split: {lines:?}");
    assert_eq!(lines[0], space_arg);
}
