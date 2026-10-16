//! Hypervisor kernel modules for the `-h` flag.
//!
//! On AMD Ryzen on AM4 (older than 7000) and the Steam Deck the LinuwUx hypervisor needs `cpuid_fault_emulation` loaded and the KVM modules unloaded: AMD-V is not designed around concurrent access from different drivers, and having both loaded at the same time crashes the system.
//! The setup therefore checks the live module state (`/sys/module`) before every step, always unloads `cpuid_fault_emulation` before touching KVM, and stops at the first failure so the unsafe combination is never created.
//! The state found before the launch is restored when the game exits, and every step runs `modprobe` as root through the first of these that can get a password: a terminal (`sudo`), a cached credential (`sudo -n`), the desktop's polkit dialog (`pkexec`), or a graphical password prompt feeding `sudo -S`.
//! Unloading KVM kills any running virtual machines.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::config::{self, App};
use crate::wrappers::find_in_path;

/// Module that emulates CPUID faulting for the hypervisor.
const CPUID_FAULT_EMULATION: &str = "cpuid_fault_emulation";
/// KVM modules to unload, in the order the guide names them; unloading `kvm_amd` pulls `kvm` with it.
const KVM_MODULES: &[&str] = &["kvm_amd", "kvm"];
/// Title of the graphical password prompt.
const DIALOG_TITLE: &str = "game: hypervisor modules";

/// Which of the relevant modules were loaded when the launcher looked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModuleState {
    /// `kvm_amd` or `kvm` was loaded.
    kvm: bool,
    /// `cpuid_fault_emulation` was loaded.
    cpuid: bool,
}

/// One `modprobe` step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// `modprobe -r` the listed modules, skipping the ones that are not loaded at execution time.
    Remove(&'static [&'static str]),
    /// `modprobe` the listed modules, skipping the ones that are already loaded at execution time.
    Load(&'static [&'static str]),
}

/// The module state found before the launch, so [`restore`] can put it back.
pub struct Session {
    before: ModuleState,
}

/// True when `/sys/module` lists `name`, i.e. the module is loaded (or built in).
fn module_loaded(name: &str) -> bool {
    Path::new("/sys/module").join(name).is_dir()
}

/// The current state of the KVM modules and `cpuid_fault_emulation`.
fn read_state() -> ModuleState {
    ModuleState {
        kvm: KVM_MODULES.iter().any(|name| module_loaded(name)),
        cpuid: module_loaded(CPUID_FAULT_EMULATION),
    }
}

/// The steps that end with `cpuid_fault_emulation` loaded and no KVM modules.
fn setup_actions(state: ModuleState) -> Vec<Action> {
    match (state.kvm, state.cpuid) {
        // The unsafe combination: unload `cpuid_fault_emulation` first, then KVM, then load it back.
        (true, true) => vec![
            Action::Remove(&[CPUID_FAULT_EMULATION]),
            Action::Remove(KVM_MODULES),
            Action::Load(&[CPUID_FAULT_EMULATION]),
        ],
        // KVM only, the common case: the init step from the guide.
        (true, false) => vec![
            Action::Remove(KVM_MODULES),
            Action::Load(&[CPUID_FAULT_EMULATION]),
        ],
        // Already prepared, leave it alone.
        (false, true) => Vec::new(),
        (false, false) => vec![Action::Load(&[CPUID_FAULT_EMULATION])],
    }
}

/// The steps that put back what was loaded before the launch.
fn restore_actions(before: ModuleState) -> Vec<Action> {
    if before.kvm {
        // KVM was running before, so it comes back; `cpuid_fault_emulation` is unloaded first and not reloaded, see [`restore`].
        vec![
            Action::Remove(&[CPUID_FAULT_EMULATION]),
            Action::Load(KVM_MODULES),
        ]
    } else if before.cpuid {
        // The setup left the modules alone, nothing to undo.
        Vec::new()
    } else {
        // Nothing was loaded before, remove what the setup loaded.
        vec![Action::Remove(&[CPUID_FAULT_EMULATION])]
    }
}

/// Run the steps, checking the live state before each one and stopping at the first failure.
///
/// The steps of one phase share a [`RootAccess`], so a password asked for once covers the whole phase.
/// Stopping on failure matters: continuing after a failed KVM unload would load `cpuid_fault_emulation` next to a KVM that is still there.
fn execute(app: &App, root: &mut RootAccess, actions: &[Action]) -> Result<(), String> {
    for action in actions {
        match action {
            Action::Remove(modules) => {
                let loaded: Vec<&str> = modules
                    .iter()
                    .copied()
                    .filter(|name| module_loaded(name))
                    .collect();
                if loaded.is_empty() {
                    app.log(&format!(
                        "Hypervisor: already unloaded: {}",
                        modules.join(", ")
                    ));
                    continue;
                }
                root.run(app, true, &loaded)?;
            }
            Action::Load(modules) => {
                let missing: Vec<&str> = modules
                    .iter()
                    .copied()
                    .filter(|name| !module_loaded(name))
                    .collect();
                if missing.is_empty() {
                    app.log(&format!(
                        "Hypervisor: already loaded: {}",
                        modules.join(", ")
                    ));
                    continue;
                }
                root.run(app, false, &missing)?;
            }
        }
    }
    Ok(())
}

/// A failed privileged call.
struct Failure {
    /// Message for the user.
    message: String,
    /// True when root rights could not be obtained, i.e. trying another way of asking might work.
    auth: bool,
}

impl Failure {
    /// A failure that another way of asking for the password cannot fix.
    fn plain(message: String) -> Failure {
        Failure {
            message,
            auth: false,
        }
    }
}

/// The ways of getting root rights, resolved on first use and tried in order until one works.
#[derive(Default)]
struct RootAccess {
    modes: Option<Vec<Mode>>,
}

impl RootAccess {
    /// Run `modprobe [-r] <modules>` as root, moving to the next way of asking when a mode cannot get a password.
    fn run(&mut self, app: &App, remove: bool, modules: &[&str]) -> Result<(), String> {
        let modes = self.modes.get_or_insert_with(candidates);
        while !modes.is_empty() {
            match run_with(app, &mut modes[0], remove, modules) {
                Ok(()) => return Ok(()),
                Err(failure) => {
                    if failure.auth && modes.len() > 1 {
                        let skipped = modes.remove(0);
                        app.log(&format!(
                            "Hypervisor: {} could not ask for the password, trying {}",
                            describe(&skipped),
                            describe(&modes[0])
                        ));
                    } else {
                        return Err(failure.message);
                    }
                }
            }
        }
        Err("no way to ask for the root password was found".to_string())
    }
}

/// How the privileged `modprobe` calls get their root rights.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// `sudo` with the inherited terminal: it asks in the terminal when a password is needed.
    Terminal,
    /// `sudo -n`: passwordless sudo or a still valid cached credential, no prompt at all.
    Cached,
    /// `pkexec`: the desktop's polkit agent shows its password dialog and keeps the authorization for a while.
    Polkit(PathBuf),
    /// `sudo -S` with the password from a graphical dialog, asked once per phase.
    Dialog {
        prompt: PasswordPrompt,
        password: Option<String>,
    },
}

/// A graphical password prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasswordPrompt {
    Zenity,
    KDialog,
    Yad,
}

impl PasswordPrompt {
    /// The first graphical password prompt installed on this machine.
    fn find() -> Option<PasswordPrompt> {
        [
            PasswordPrompt::Zenity,
            PasswordPrompt::KDialog,
            PasswordPrompt::Yad,
        ]
        .into_iter()
        .find(|dialog| find_in_path(dialog.program()).is_some())
    }

    /// Program name of the prompt.
    fn program(self) -> &'static str {
        match self {
            PasswordPrompt::Zenity => "zenity",
            PasswordPrompt::KDialog => "kdialog",
            PasswordPrompt::Yad => "yad",
        }
    }

    /// Arguments that make the program ask for a password and print it to stdout.
    fn args(self, title: &str) -> Vec<String> {
        match self {
            PasswordPrompt::Zenity | PasswordPrompt::Yad => {
                vec!["--password".to_string(), format!("--title={title}")]
            }
            PasswordPrompt::KDialog => vec!["--password".to_string(), title.to_string()],
        }
    }
}

/// The ways of getting root rights this machine can use, best first.
///
/// A terminal is preferred because sudo then asks there without any popup; otherwise a cached credential avoids a prompt entirely, then pkexec's polkit dialog, then a graphical password prompt.
fn candidates() -> Vec<Mode> {
    let terminal = std::io::stdin().is_terminal();
    let mut modes = Vec::new();
    if terminal {
        modes.push(Mode::Terminal);
    } else if sudo_cached() {
        modes.push(Mode::Cached);
    }
    if let Some(pkexec) = find_in_path("pkexec") {
        modes.push(Mode::Polkit(pkexec));
    }
    if let Some(prompt) = PasswordPrompt::find() {
        modes.push(Mode::Dialog {
            prompt,
            password: None,
        });
    }
    if modes.is_empty() {
        // Nothing else is available, keep plain sudo so its own error message is reported.
        modes.push(Mode::Terminal);
    }
    modes
}

/// True when `sudo -n` can run anything right now.
fn sudo_cached() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// How `sudo` is told to get the password.
enum SudoPrompt {
    /// Read from the inherited terminal.
    Terminal,
    /// Never ask (`-n`).
    Disabled,
    /// Read from the piped stdin (`-S`), with the prompt silenced.
    Piped(String),
}

/// One privileged call with `mode`, reporting whether another mode could get a password where this one could not.
fn run_with(app: &App, mode: &mut Mode, remove: bool, modules: &[&str]) -> Result<(), Failure> {
    match mode {
        Mode::Terminal => run_sudo(app, SudoPrompt::Terminal, remove, modules),
        Mode::Cached => run_sudo(app, SudoPrompt::Disabled, remove, modules),
        Mode::Polkit(pkexec) => run_pkexec(app, pkexec, remove, modules),
        Mode::Dialog { prompt, password } => {
            // A still valid sudo timestamp needs no dialog at all.
            match run_sudo(app, SudoPrompt::Disabled, remove, modules) {
                Ok(()) => return Ok(()),
                Err(failure) if failure.auth => {}
                Err(failure) => return Err(failure),
            }
            if password.is_none() {
                *password = Some(ask_password(app, *prompt)?);
            }
            let secret = password.clone().unwrap_or_default();
            run_sudo(app, SudoPrompt::Piped(secret), remove, modules)
        }
    }
}

/// `sudo modprobe [-r] <modules>` with the password handling of `prompt`.
fn run_sudo(app: &App, prompt: SudoPrompt, remove: bool, modules: &[&str]) -> Result<(), Failure> {
    let mut command = Command::new("sudo");
    let flags = match &prompt {
        SudoPrompt::Terminal => "",
        SudoPrompt::Disabled => "-n ",
        SudoPrompt::Piped(_) => "-S ",
    };
    match &prompt {
        SudoPrompt::Terminal => {}
        SudoPrompt::Disabled => {
            command.arg("-n");
        }
        SudoPrompt::Piped(_) => {
            command.args(["-S", "-p", ""]);
        }
    }
    command.arg("modprobe");
    if remove {
        command.arg("-r");
    }
    command.args(modules);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command.stdin(match &prompt {
        SudoPrompt::Terminal => Stdio::inherit(),
        SudoPrompt::Disabled => Stdio::null(),
        SudoPrompt::Piped(_) => Stdio::piped(),
    });

    let shown = format!(
        "sudo {flags}modprobe {}{}",
        if remove { "-r " } else { "" },
        modules.join(" ")
    );
    app.log(&format!("Hypervisor: running {shown}"));

    let mut child = command.spawn().map_err(|e| Failure {
        message: format!("could not run {shown}: {e}"),
        // Another way of asking may still work, e.g. pkexec where sudo is not installed.
        auth: true,
    })?;

    if let (SudoPrompt::Piped(secret), Some(mut stdin)) = (&prompt, child.stdin.take()) {
        // Write errors are ignored on purpose: a rejected password makes sudo exit, and the output below says why.
        let _ = stdin.write_all(secret.as_bytes());
        let _ = stdin.write_all(b"\n");
    }

    let out = child
        .wait_with_output()
        .map_err(|e| Failure::plain(format!("could not run {shown}: {e}")))?;

    finish(app, &shown, out, sudo_needs_a_password)
}

/// `pkexec modprobe [-r] <modules>` with the modprobe path made absolute, which is what pkexec expects.
fn run_pkexec(app: &App, pkexec: &Path, remove: bool, modules: &[&str]) -> Result<(), Failure> {
    let modprobe = find_in_path("modprobe")
        .ok_or_else(|| Failure::plain("modprobe was not found on PATH".to_string()))?;
    let mut command = Command::new(pkexec);
    command.arg(&modprobe);
    if remove {
        command.arg("-r");
    }
    command.args(modules);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let shown = format!(
        "pkexec {} {}{}",
        modprobe.display(),
        if remove { "-r " } else { "" },
        modules.join(" ")
    );
    app.log(&format!("Hypervisor: running {shown}"));

    let out = command.output().map_err(|e| Failure {
        message: format!("could not run {shown}: {e}"),
        auth: true,
    })?;

    finish(app, &shown, out, pkexec_needs_authentication)
}

/// Ask for the root password with a graphical dialog; a dismissed dialog or an empty password is an auth failure.
fn ask_password(app: &App, prompt: PasswordPrompt) -> Result<String, Failure> {
    let Some(program) = find_in_path(prompt.program()) else {
        return Err(Failure {
            message: format!("{} was not found on PATH", prompt.program()),
            auth: true,
        });
    };
    let args = prompt.args(DIALOG_TITLE);
    let shown = format!("{} {}", prompt.program(), args.join(" "));
    app.log(&format!(
        "Hypervisor: asking for the root password with {shown}"
    ));

    let out = Command::new(&program)
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Failure {
            message: format!("could not run {shown}: {e}"),
            auth: true,
        })?;

    if !out.status.success() {
        return Err(Failure {
            message: format!("{shown} was dismissed without a password"),
            auth: true,
        });
    }
    let password = String::from_utf8_lossy(&out.stdout)
        .trim_end_matches(['\n', '\r'])
        .to_string();
    if password.is_empty() {
        return Err(Failure {
            message: format!("{shown} returned an empty password"),
            auth: true,
        });
    }
    Ok(password)
}

/// Log the output of a privileged call and turn a non-zero exit into a failure classified by `classify`.
fn finish(
    app: &App,
    shown: &str,
    out: Output,
    classify: fn(Option<i32>, &str) -> bool,
) -> Result<(), Failure> {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for line in stdout.lines().chain(stderr.lines()) {
        if !line.trim().is_empty() {
            app.log(&format!("Hypervisor: {line}"));
        }
    }
    if out.status.success() {
        return Ok(());
    }
    let detail = if stderr.trim().is_empty() {
        stdout.trim().to_string()
    } else {
        stderr.trim().to_string()
    };
    let message = if detail.is_empty() {
        format!("{shown} exited with {}", out.status)
    } else {
        format!("{shown} failed: {detail}")
    };
    Err(Failure {
        message,
        auth: classify(out.status.code(), &stderr),
    })
}

/// True when sudo's output means it could not get a password, so another way of asking might work.
fn sudo_needs_a_password(_status: Option<i32>, stderr: &str) -> bool {
    const MARKERS: &[&str] = &[
        "a terminal is required",
        "a password is required",
        "no password",
        "incorrect password",
        "try again",
        "not in the sudoers file",
        "not allowed to execute",
    ];
    MARKERS.iter().any(|marker| stderr.contains(marker))
}

/// True when pkexec's output means polkit could not authenticate, so another way of asking might work.
///
/// pkexec exits with 127 when the user is not authorized or authentication failed, and with 126 when the dialog was dismissed.
fn pkexec_needs_authentication(status: Option<i32>, stderr: &str) -> bool {
    if matches!(status, Some(126) | Some(127)) {
        return true;
    }
    let stderr = stderr.to_lowercase();
    ["authentication", "authoriz", "dismiss"]
        .iter()
        .any(|marker| stderr.contains(marker))
}

/// Name of a way of getting root rights, for the log.
fn describe(mode: &Mode) -> String {
    match mode {
        Mode::Terminal => "sudo".to_string(),
        Mode::Cached => "sudo -n".to_string(),
        Mode::Polkit(_) => "pkexec".to_string(),
        Mode::Dialog { prompt, .. } => prompt.program().to_string(),
    }
}

/// Set the modules up before the game (the `-h` flag).
///
/// Returns the session for [`restore`], or `None` when the flag is off or the module state cannot be read.
/// A failed step is rolled back to the state found before the launch, notified, and is not fatal: the game still starts.
pub fn prepare(app: &App) -> Option<Session> {
    if !app.hypervisor_modules {
        return None;
    }
    if !Path::new("/sys/module").is_dir() {
        app.log("Hypervisor: /sys/module is not readable, skipping the module setup");
        return None;
    }

    let before = read_state();
    app.log(&format!(
        "Hypervisor: found {CPUID_FAULT_EMULATION}={}, kvm={} before the launch",
        before.cpuid, before.kvm
    ));

    let mut root = RootAccess::default();
    if let Err(msg) = execute(app, &mut root, &setup_actions(before)) {
        app.log(&format!("Hypervisor: setup failed: {msg}"));
        eprintln!("game: hypervisor setup failed: {msg}");
        config::notify("game: hypervisor setup failed", &msg);
        if let Err(rollback) = execute(app, &mut root, &restore_actions(before)) {
            app.log(&format!("Hypervisor: rollback failed: {rollback}"));
            eprintln!("game: hypervisor rollback failed: {rollback}");
            config::notify("game: hypervisor rollback failed", &rollback);
        }
    }

    // The session is returned even after a failure, so the exit path retries the restore.
    Some(Session { before })
}

/// Undo [`prepare`] after the game exits.
///
/// KVM comes back when it was loaded before, and `cpuid_fault_emulation` is unloaded again.
/// When both were loaded before the launch, only KVM is brought back: recreating that combination, the one the guide warns about, is what the setup exists to avoid.
pub fn restore(app: &App, session: Option<Session>) {
    let Some(session) = session else {
        return;
    };
    let before = session.before;

    let actions = restore_actions(before);
    if actions.is_empty() {
        return;
    }

    if before.kvm && before.cpuid {
        app.log(&format!(
            "Hypervisor: {CPUID_FAULT_EMULATION} and KVM were both loaded before the launch, leaving {CPUID_FAULT_EMULATION} unloaded instead of restoring the unsafe combination"
        ));
    }

    let mut root = RootAccess::default();
    if let Err(msg) = execute(app, &mut root, &actions) {
        app.log(&format!("Hypervisor: restore failed: {msg}"));
        eprintln!("game: hypervisor restore failed: {msg}");
        config::notify("game: hypervisor restore failed", &msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_with_kvm_running_unloads_it_and_loads_the_module() {
        assert_eq!(
            setup_actions(ModuleState {
                kvm: true,
                cpuid: false,
            }),
            vec![
                Action::Remove(KVM_MODULES),
                Action::Load(&[CPUID_FAULT_EMULATION]),
            ]
        );
    }

    #[test]
    fn setup_from_the_unsafe_combination_unloads_cpuid_first() {
        assert_eq!(
            setup_actions(ModuleState {
                kvm: true,
                cpuid: true,
            }),
            vec![
                Action::Remove(&[CPUID_FAULT_EMULATION]),
                Action::Remove(KVM_MODULES),
                Action::Load(&[CPUID_FAULT_EMULATION]),
            ]
        );
    }

    #[test]
    fn setup_with_neither_loaded_only_loads_the_module() {
        assert_eq!(
            setup_actions(ModuleState {
                kvm: false,
                cpuid: false,
            }),
            vec![Action::Load(&[CPUID_FAULT_EMULATION])]
        );
    }

    #[test]
    fn setup_with_only_the_module_loaded_does_nothing() {
        assert!(setup_actions(ModuleState {
            kvm: false,
            cpuid: true,
        })
        .is_empty());
    }

    #[test]
    fn restore_brings_kvm_back_and_leaves_the_module_unloaded() {
        // KVM was running before: it comes back, the module does not.
        assert_eq!(
            restore_actions(ModuleState {
                kvm: true,
                cpuid: false,
            }),
            vec![
                Action::Remove(&[CPUID_FAULT_EMULATION]),
                Action::Load(KVM_MODULES),
            ]
        );
        // Both were running before: only KVM comes back, the unsafe combination is not recreated.
        assert_eq!(
            restore_actions(ModuleState {
                kvm: true,
                cpuid: true,
            }),
            vec![
                Action::Remove(&[CPUID_FAULT_EMULATION]),
                Action::Load(KVM_MODULES),
            ]
        );
        // Nothing was loaded before: the module the setup loaded is removed.
        assert_eq!(
            restore_actions(ModuleState {
                kvm: false,
                cpuid: false,
            }),
            vec![Action::Remove(&[CPUID_FAULT_EMULATION])]
        );
        // The setup did not touch a machine that already had only the module loaded.
        assert!(restore_actions(ModuleState {
            kvm: false,
            cpuid: true,
        })
        .is_empty());
    }

    /// Apply the steps to a simulated state, failing the test on any step that leaves both modules loaded at the same time.
    fn apply(actions: &[Action], mut state: ModuleState) -> ModuleState {
        for action in actions {
            match action {
                Action::Remove(modules) => {
                    if modules.contains(&CPUID_FAULT_EMULATION) {
                        state.cpuid = false;
                    }
                    if modules.iter().any(|name| KVM_MODULES.contains(name)) {
                        state.kvm = false;
                    }
                }
                Action::Load(modules) => {
                    if modules.contains(&CPUID_FAULT_EMULATION) {
                        state.cpuid = true;
                    }
                    if modules.iter().any(|name| KVM_MODULES.contains(name)) {
                        state.kvm = true;
                    }
                }
            }
            assert!(
                !(state.kvm && state.cpuid),
                "the unsafe combination was created by {action:?}: {state:?}"
            );
        }
        state
    }

    #[test]
    fn no_plan_ever_leaves_both_loaded() {
        for kvm in [false, true] {
            for cpuid in [false, true] {
                let before = ModuleState { kvm, cpuid };
                let during = apply(&setup_actions(before), before);
                let after = apply(&restore_actions(before), during);
                // KVM is running again whenever it was running before.
                assert_eq!(after.kvm, before.kvm, "kvm not restored for {before:?}");
                // The module is only still loaded when the launcher found it loaded and KVM was not running.
                assert_eq!(
                    after.cpuid,
                    before.cpuid && !before.kvm,
                    "cpuid_fault_emulation left wrong for {before:?}"
                );
            }
        }
    }

    #[test]
    fn dialogs_ask_for_a_password_on_stdout() {
        assert_eq!(
            PasswordPrompt::Zenity.args("t"),
            vec!["--password", "--title=t"]
        );
        assert_eq!(PasswordPrompt::KDialog.args("t"), vec!["--password", "t"]);
        assert_eq!(
            PasswordPrompt::Yad.args("t"),
            vec!["--password", "--title=t"]
        );
        assert_eq!(PasswordPrompt::Zenity.program(), "zenity");
    }

    #[test]
    fn sudo_password_problems_are_recognized() {
        // The error that made the GUI prompt necessary in the first place.
        assert!(sudo_needs_a_password(
            Some(1),
            "sudo: a terminal is required to read the password; either use the -S option to read from standard input or configure an askpass helper\nsudo: a password is required"
        ));
        assert!(sudo_needs_a_password(
            Some(1),
            "sudo: 3 incorrect password attempts"
        ));
        assert!(sudo_needs_a_password(
            Some(1),
            "sudo: user is not in the sudoers file.  This incident will be reported."
        ));
        assert!(sudo_needs_a_password(
            Some(1),
            "sudo: user is not allowed to execute '/usr/bin/modprobe' as root"
        ));
        // Real modprobe failures must not trigger another password prompt.
        assert!(!sudo_needs_a_password(
            Some(1),
            "modprobe: FATAL: Module kvm_amd is in use."
        ));
        assert!(!sudo_needs_a_password(
            Some(1),
            "modprobe: ERROR: could not insert 'cpuid_fault_emulation': Operation not permitted"
        ));
        assert!(!sudo_needs_a_password(Some(1), ""));
    }

    #[test]
    fn pkexec_authentication_problems_are_recognized() {
        // 126: the polkit dialog was dismissed, 127: not authorized or no agent to ask.
        assert!(pkexec_needs_authentication(Some(126), ""));
        assert!(pkexec_needs_authentication(Some(127), ""));
        assert!(pkexec_needs_authentication(
            Some(127),
            "Error executing command as another user: No authentication agent found."
        ));
        // Real modprobe failures come through with modprobe's own exit code.
        assert!(!pkexec_needs_authentication(
            Some(1),
            "modprobe: FATAL: Module kvm_amd is in use."
        ));
    }
}
