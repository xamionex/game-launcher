//! Interactive config editor (the `-C` flag) and launch options generator (the `-k` flag).
//!
//! A ratatui form over the TOML document: booleans toggle with space or enter, values are edited inline, and lists open a sub-editor where entries can be added, edited and removed.
//! Saving rewrites the config file through `toml_edit`, so comments and formatting survive.
//! Both flags are actions: the editor exits after saving or discarding, the generator exits after copying the launch options line, and neither launches a game.
//! Ctrl+C leaves either of them at once, without saving and without copying.

use std::io::IsTerminal;
use std::path::PathBuf;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use toml_edit::{Array, DocumentMut, Item, Value};

use crate::config_file;
use crate::wrappers::Monitor;

/// One editable setting in the form.
struct Field {
    key: &'static str,
    label: &'static str,
    flag: &'static str,
    help: &'static str,
    section: &'static str,
}

/// Settings that cycle through fixed values instead of being typed in.
fn choices_for(key: &str) -> &'static [i64] {
    match key {
        "logging_level" => &[-1, 0, 1],
        _ => &[],
    }
}

/// Example entry shown while editing a list setting.
fn list_example(key: &str) -> &'static str {
    match key {
        "dll_overrides" => "dinput8=n,b",
        "mods" => "./mod-loader.sh",
        "exports" => "PROTON_NO_ESYNC=1",
        _ => "",
    }
}

/// The settings shown by the editor, in display order.
///
/// `field_keys_match_default_config` keeps this table in sync with [`config_file::DEFAULT_CONFIG`].
const FIELDS: &[Field] = &[
    Field {
        key: "gamemode",
        label: "GameMode",
        flag: "-g",
        help: "Skipped on BORE kernels or with ananicy-cpp running",
        section: "Enabled by default",
    },
    Field {
        key: "mangohud",
        label: "MangoHud",
        flag: "-h",
        help: "Overlay; the flag disables it",
        section: "Enabled by default",
    },
    Field {
        key: "mangohud_force",
        label: "MangoHud force",
        flag: "-H",
        help: "Force MangoHud on in gaming mode",
        section: "Enabled by default",
    },
    Field {
        key: "protonhax",
        label: "ProtonHax",
        flag: "-p",
        help: "Proton launch hooks; the flag disables them",
        section: "Enabled by default",
    },
    Field {
        key: "wayland_force_enable",
        label: "Wayland force on",
        flag: "-W",
        help: "Force Wayland regardless of GPU vendor",
        section: "Enabled by default",
    },
    Field {
        key: "wayland_force_disable",
        label: "Wayland force off",
        flag: "-X",
        help: "Force Wayland off",
        section: "Enabled by default",
    },
    Field {
        key: "pressure_vessel",
        label: "Pressure Vessel",
        flag: "-P",
        help: "Strip the Steam Linux Runtime",
        section: "Disabled by default",
    },
    Field {
        key: "disable_sdl3",
        label: "SDL3 elimination",
        flag: "-L",
        help: "Sets STEAM_COMPAT_RUNTIME_SDL3=0",
        section: "Disabled by default",
    },
    Field {
        key: "gamescope",
        label: "Gamescope X11",
        flag: "-s",
        help: "Gamescope with the X11 backend",
        section: "Disabled by default",
    },
    Field {
        key: "gamescope_wayland",
        label: "Gamescope Wayland",
        flag: "-S",
        help: "Gamescope with the Wayland backend",
        section: "Disabled by default",
    },
    Field {
        key: "wezterm",
        label: "wezterm",
        flag: "-w",
        help: "Run the game inside wezterm",
        section: "Disabled by default",
    },
    Field {
        key: "onlinefix",
        label: "OnlineFix",
        flag: "-o",
        help: "OnlineFix DLL overrides",
        section: "Disabled by default",
    },
    Field {
        key: "cleanup_mods_on_exit",
        label: "Cleanup mods",
        flag: "-e",
        help: "Kill mod processes when the launcher exits",
        section: "Disabled by default",
    },
    Field {
        key: "lsfg",
        label: "LSFG-VK",
        flag: "-f",
        help: "LSFG-VK frame generation",
        section: "Disabled by default",
    },
    Field {
        key: "modding_support",
        label: "Modding support",
        flag: "-m",
        help: "Modding DLL overrides",
        section: "Disabled by default",
    },
    Field {
        key: "fix_audit",
        label: "netsock LD_AUDIT",
        flag: "-F",
        help: "Cached netsock loader in LD_AUDIT",
        section: "Disabled by default",
    },
    Field {
        key: "hypervisor",
        label: "Hypervisor",
        flag: "-v",
        help: "LinuwUx loader in LD_PRELOAD",
        section: "Disabled by default",
    },
    Field {
        key: "enable_custom_vkd3d",
        label: "Custom vkd3d",
        flag: "-V",
        help: "Use the build in ~/Projects/vkd3d-proton",
        section: "Disabled by default",
    },
    Field {
        key: "eos_proxy",
        label: "EOS-Proxy",
        flag: "-E",
        help: "Swap the game's EOSSDK-Win64-Shipping.dll",
        section: "Disabled by default",
    },
    Field {
        key: "logging_level",
        label: "Log level",
        flag: "-l",
        help: "Integer: -1 silent, 0 normal, 1 verbose",
        section: "Values",
    },
    Field {
        key: "instances",
        label: "Instances",
        flag: "-i",
        help: "Accepted, currently inert",
        section: "Values",
    },
    Field {
        key: "replacement_exe",
        label: "Replacement exe",
        flag: "-r",
        help: "Replace the launched executable",
        section: "Values",
    },
    Field {
        key: "wayland_monitor",
        label: "Wayland monitor",
        flag: "-M",
        help: "Primary monitor for the Wine Wayland driver (WAYLANDDRV_PRIMARY_MONITOR)",
        section: "Wayland",
    },
    Field {
        key: "dll_overrides",
        label: "DLL overrides",
        flag: "-d",
        help: "WINEDLLOVERRIDES entries",
        section: "Lists",
    },
    Field {
        key: "mods",
        label: "Mods",
        flag: "-u",
        help: "Background mod commands",
        section: "Lists",
    },
    Field {
        key: "exports",
        label: "Exports",
        flag: "",
        help: "NAME=VALUE environment exports",
        section: "Lists",
    },
];

/// Kind of a setting, taken from the value in the default document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Bool,
    Integer,
    Text,
    List,
}

impl Kind {
    fn of(item: &Item) -> Kind {
        if item.is_bool() {
            Kind::Bool
        } else if item.is_integer() {
            Kind::Integer
        } else if item.is_array() {
            Kind::List
        } else {
            Kind::Text
        }
    }
}

/// A row in the form: a section heading or a selectable field.
enum Entry {
    Section(&'static str),
    Field(usize),
}

/// Inline text editing state.
struct TextEdit {
    key: String,
    /// Index of the list entry being edited, when a list entry is edited.
    entry: Option<usize>,
    /// True when the edit inserts a new entry at `entry` instead of replacing one.
    adding: bool,
    buffer: String,
    /// Cursor position as a character index into `buffer`.
    cursor: usize,
}

/// What the editor is for.
enum Purpose {
    /// `-C`: the document is the config file and `s` writes it back.
    Config,
    /// `-k`: the document holds the choices, the loaded config is kept as the base, and only the differences become launch flags.
    LaunchOptions { base: DocumentMut },
}

/// Editor mode.
enum Mode {
    Form,
    EditText(TextEdit),
    EditList,
}

struct Editor {
    doc: DocumentMut,
    path: PathBuf,
    mode: Mode,
    /// Whether this run edits the config file or generates a launch options line.
    purpose: Purpose,
    /// Set by Ctrl+C: leave without saving and without copying.
    cancelled: bool,
    /// Path of the running binary, offered instead of `game` in the generated line.
    binary: String,
    /// Put the absolute binary path in the generated line.
    absolute: bool,
    /// Index into the entries vector built from [`FIELDS`].
    selection: usize,
    list_selection: usize,
    dirty: bool,
    confirm_quit: bool,
    status: String,
    /// Wayland outputs detected on this machine.
    monitors: Vec<Monitor>,
    /// Output the editor's own terminal is on, when the compositor knows.
    active_monitor: Option<String>,
    /// Whether the launcher would enable Wayland based on the GPU alone.
    gpu_wayland: bool,
}

impl Editor {
    fn new(
        path: PathBuf,
        doc: DocumentMut,
        monitors: Vec<Monitor>,
        active_monitor: Option<String>,
        gpu_wayland: bool,
    ) -> Editor {
        Editor::with_purpose(
            path,
            doc,
            monitors,
            active_monitor,
            gpu_wayland,
            Purpose::Config,
        )
    }

    /// The `-k` editor: the loaded config is the base, and what the user changes becomes launch flags.
    fn new_generator(
        path: PathBuf,
        doc: DocumentMut,
        monitors: Vec<Monitor>,
        active_monitor: Option<String>,
        gpu_wayland: bool,
    ) -> Editor {
        let base = doc.clone();
        Editor::with_purpose(
            path,
            doc,
            monitors,
            active_monitor,
            gpu_wayland,
            Purpose::LaunchOptions { base },
        )
    }

    fn with_purpose(
        path: PathBuf,
        doc: DocumentMut,
        monitors: Vec<Monitor>,
        active_monitor: Option<String>,
        gpu_wayland: bool,
        purpose: Purpose,
    ) -> Editor {
        Editor {
            doc,
            path,
            mode: Mode::Form,
            purpose,
            cancelled: false,
            binary: std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "game".to_string()),
            absolute: false,
            selection: 0,
            list_selection: 0,
            dirty: false,
            confirm_quit: false,
            status: String::new(),
            monitors,
            active_monitor,
            gpu_wayland,
        }
    }

    /// True while the `-k` generator is running.
    fn is_generator(&self) -> bool {
        matches!(self.purpose, Purpose::LaunchOptions { .. })
    }

    // ---- document access ----

    fn kind(&self, key: &str) -> Kind {
        self.doc.get(key).map(Kind::of).unwrap_or(Kind::Text)
    }

    fn get_bool(&self, key: &str) -> bool {
        self.doc.get(key).and_then(Item::as_bool).unwrap_or(false)
    }

    fn get_integer(&self, key: &str) -> i64 {
        self.doc.get(key).and_then(Item::as_integer).unwrap_or(0)
    }

    fn get_text(&self, key: &str) -> String {
        self.doc
            .get(key)
            .and_then(Item::as_str)
            .unwrap_or_default()
            .to_string()
    }

    fn get_list(&self, key: &str) -> Vec<String> {
        self.doc
            .get(key)
            .and_then(Item::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Replace the value of `key`, keeping the comments around it.
    fn set_value(&mut self, key: &str, new: Value) {
        match self.doc.get_mut(key).and_then(Item::as_value_mut) {
            Some(value) => {
                let decor = value.decor().clone();
                *value = new;
                *value.decor_mut() = decor;
            }
            None => {
                self.doc[key] = Item::Value(new);
            }
        }
        self.dirty = true;
    }

    fn set_list(&mut self, key: &str, items: &[String]) {
        let mut array = Array::new();
        for item in items {
            array.push(item.as_str());
        }
        self.set_value(key, Value::Array(array));
    }

    // ---- entries and selection ----

    fn entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        let mut section = "";
        for (i, field) in FIELDS.iter().enumerate() {
            if !self.is_visible(field.key) {
                continue;
            }
            if field.section != section {
                section = field.section;
                entries.push(Entry::Section(section));
            }
            entries.push(Entry::Field(i));
        }
        entries
    }

    /// The Wayland monitor row only exists when Wayland is in use.
    fn is_visible(&self, key: &str) -> bool {
        key != "wayland_monitor" || self.wayland_active()
    }

    /// Whether the launcher would enable Wayland on this system, honoring the force flags from the document.
    fn wayland_active(&self) -> bool {
        if self.get_bool("wayland_force_enable") {
            return true;
        }
        if self.get_bool("wayland_force_disable") {
            return false;
        }
        self.gpu_wayland
    }

    /// Selectable values for the Wayland monitor: unset plus every detected output.
    /// A monitor that is configured but not currently detected is kept, so cycling never silently drops it.
    fn monitor_choices(&self) -> Vec<String> {
        let mut choices = vec![String::new()];
        for monitor in &self.monitors {
            if !choices.contains(&monitor.name) {
                choices.push(monitor.name.clone());
            }
        }
        let current = self.get_text("wayland_monitor");
        if !current.is_empty() && !choices.contains(&current) {
            choices.push(current);
        }
        choices
    }

    /// Ask the compositor which output this terminal is on.
    fn refresh_active_monitor(&mut self) {
        self.active_monitor = crate::wrappers::active_monitor();
    }

    /// Read only note shown next to the picker: where this editor is running.
    fn terminal_note(&self) -> Option<String> {
        self.active_monitor
            .as_deref()
            .map(|name| format!("this terminal: {name}"))
    }

    /// Monitor list with the terminal's own output marked, shown in the footer.
    fn monitor_summary(&self) -> String {
        if self.monitors.is_empty() {
            return "No monitors detected".to_string();
        }
        let parts: Vec<String> = self
            .monitors
            .iter()
            .map(|monitor| {
                let mut text = monitor.name.clone();
                if !monitor.detail.is_empty() {
                    text.push_str(&format!(" ({})", monitor.detail));
                }
                if self.active_monitor.as_deref() == Some(monitor.name.as_str()) {
                    text.push_str(" [this terminal]");
                }
                text
            })
            .collect();
        format!("Monitors: {}", parts.join("  |  "))
    }

    /// Move the Wayland monitor through the detected outputs.
    fn step_monitor(&mut self, delta: i64) {
        self.refresh_active_monitor();
        let choices = self.monitor_choices();
        if choices.len() <= 1 {
            self.status = "No Wayland monitors detected".to_string();
            return;
        }
        let current = self.get_text("wayland_monitor");
        let index = choices.iter().position(|c| *c == current).unwrap_or(0) as i64;
        let len = choices.len() as i64;
        let next = (index + delta).rem_euclid(len) as usize;
        let value = choices[next].clone();
        self.set_value("wayland_monitor", Value::from(value));
    }

    /// Index of the currently selected field in [`FIELDS`].
    fn selected_field(&self) -> usize {
        match self.entries().get(self.selection) {
            Some(Entry::Field(i)) => *i,
            _ => 0,
        }
    }

    /// Move the selection by `delta` rows, skipping section headings.
    fn move_selection(&mut self, delta: isize) {
        let entries = self.entries();
        let count = entries.len();
        if count == 0 {
            return;
        }
        let mut index = self.selection as isize + delta;
        while index >= 0 && (index as usize) < count {
            if matches!(entries[index as usize], Entry::Field(_)) {
                self.selection = index as usize;
                return;
            }
            index += delta.signum();
        }
        if delta < 0 {
            self.jump_to_first();
        } else {
            self.jump_to_last();
        }
    }

    fn jump_to_first(&mut self) {
        if let Some(index) = self
            .entries()
            .iter()
            .position(|e| matches!(e, Entry::Field(_)))
        {
            self.selection = index;
        }
    }

    fn jump_to_last(&mut self) {
        if let Some(index) = self
            .entries()
            .iter()
            .rposition(|e| matches!(e, Entry::Field(_)))
        {
            self.selection = index;
        }
    }

    /// Move the selection to the row for `key` (used by tests).
    #[cfg(test)]
    fn select_key(&mut self, key: &str) {
        for (row, entry) in self.entries().iter().enumerate() {
            if let Entry::Field(index) = entry {
                if FIELDS[*index].key == key {
                    self.selection = row;
                    return;
                }
            }
        }
    }

    /// Keys currently shown by the form (used by tests).
    #[cfg(test)]
    fn visible_keys(&self) -> Vec<&'static str> {
        self.entries()
            .iter()
            .filter_map(|entry| match entry {
                Entry::Field(index) => Some(FIELDS[*index].key),
                Entry::Section(_) => None,
            })
            .collect()
    }

    /// Keep the cursor on a selectable row, e.g. after a row was hidden.
    fn normalize_selection(&mut self) {
        if matches!(self.entries().get(self.selection), Some(Entry::Field(_))) {
            return;
        }
        self.jump_to_first();
    }

    // ---- actions ----

    /// Toggle a boolean, cycle a choice, or open the right editor.
    fn activate(&mut self) {
        let field = &FIELDS[self.selected_field()];
        if field.key == "wayland_monitor" {
            self.step_monitor(1);
            return;
        }
        match self.kind(field.key) {
            Kind::Bool => {
                let value = self.get_bool(field.key);
                self.set_value(field.key, Value::from(!value));
            }
            Kind::Integer if !choices_for(field.key).is_empty() => {
                let choices = choices_for(field.key);
                let current = self.get_integer(field.key);
                let index = choices.iter().position(|c| *c == current).unwrap_or(0);
                self.set_value(field.key, Value::from(choices[(index + 1) % choices.len()]));
            }
            Kind::Integer => {
                let buffer = self.get_integer(field.key).to_string();
                self.mode = Mode::EditText(TextEdit {
                    key: field.key.to_string(),
                    entry: None,
                    adding: false,
                    cursor: buffer.chars().count(),
                    buffer,
                });
            }
            Kind::List => {
                self.mode = Mode::EditList;
                self.list_selection = 0;
            }
            Kind::Text => {
                self.mode = Mode::EditText(TextEdit {
                    key: field.key.to_string(),
                    entry: None,
                    adding: false,
                    buffer: self.get_text(field.key),
                    cursor: self.get_text(field.key).chars().count(),
                });
            }
        }
    }

    fn save(&mut self) -> Result<(), String> {
        match config_file::save_document(&self.doc, &self.path) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved to {}", self.path.display());
                Ok(())
            }
            Err(e) => {
                self.status = format!("Save failed: {e}");
                Err(e)
            }
        }
    }

    // ---- launch options (-k) ----

    /// Program name used by the generated line.
    fn program(&self) -> String {
        if self.absolute {
            self.binary.clone()
        } else {
            "game".to_string()
        }
    }

    /// Flags for everything the user changed, plus a note for changes no flag can express.
    fn launch_flags(&self) -> (Vec<String>, Vec<String>) {
        let Purpose::LaunchOptions { base } = &self.purpose else {
            return (Vec::new(), Vec::new());
        };
        let defaults = config_file::default_document();
        let mut flags: Vec<String> = Vec::new();
        let mut notes: Vec<String> = Vec::new();

        for field in FIELDS {
            let key = field.key;
            let Some(default_item) = defaults.get(key) else {
                continue;
            };
            match Kind::of(default_item) {
                Kind::Bool => {
                    let target = self.get_bool(key);
                    let base_value = base.get(key).and_then(Item::as_bool).unwrap_or(false);
                    if target == base_value {
                        continue;
                    }
                    let built_in = default_item.as_bool().unwrap_or(false);
                    if target != built_in && !field.flag.is_empty() {
                        // The flag flips the built-in default, which is exactly what the change asks for.
                        flags.push(field.flag.to_string());
                    } else {
                        notes.push(format!(
                            "{key}: your config sets this and no flag covers it, use -C"
                        ));
                    }
                }
                Kind::Integer => {
                    let target = self.get_integer(key);
                    let base_value = base.get(key).and_then(Item::as_integer).unwrap_or(0);
                    if target == base_value || field.flag.is_empty() {
                        continue;
                    }
                    flags.push(format!("{} {}", field.flag, target));
                }
                Kind::Text => {
                    let target = self.get_text(key);
                    let base_value = base
                        .get(key)
                        .and_then(Item::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if target == base_value || field.flag.is_empty() || target.is_empty() {
                        continue;
                    }
                    flags.push(format!("{} {}", field.flag, shell_quote(&target)));
                }
                Kind::List => {
                    let target = self.get_list(key);
                    let base_list = base
                        .get(key)
                        .and_then(Item::as_array)
                        .map(|array| {
                            array
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect::<Vec<String>>()
                        })
                        .unwrap_or_default();
                    let added: Vec<String> = target
                        .iter()
                        .filter(|item| !base_list.contains(*item))
                        .cloned()
                        .collect();
                    if base_list.iter().any(|item| !target.contains(item)) {
                        notes.push(format!(
                            "{key}: a flag can only add entries, use -C to remove one"
                        ));
                    }
                    if added.is_empty() {
                        continue;
                    }
                    match key {
                        // Environment exports are positional tokens, not a flag.
                        "exports" => flags.extend(added.iter().map(|item| shell_quote(item))),
                        // DLL overrides travel as one semicolon separated argument.
                        "dll_overrides" => {
                            flags.push(format!("{} {}", field.flag, shell_quote(&added.join(";"))))
                        }
                        _ => flags.extend(
                            added
                                .iter()
                                .map(|item| format!("{} {}", field.flag, shell_quote(item))),
                        ),
                    }
                }
            }
        }
        (flags, notes)
    }

    /// The launch options line as it would be pasted into Steam.
    fn generated_line(&self) -> String {
        let (flags, _) = self.launch_flags();
        let mut parts = vec![self.program()];
        parts.extend(flags);
        parts.push("--".to_string());
        parts.push("%command%".to_string());
        parts.join(" ")
    }

    /// Copy the current line without leaving the generator.
    fn copy_now(&mut self) {
        let line = self.generated_line();
        self.status = match copy_to_clipboard(&line) {
            Ok(tool) => format!("Copied to the clipboard with {tool}"),
            Err(e) => format!("Could not copy: {e}"),
        };
    }

    // ---- input ----

    /// Handle one key press; returns `true` when the editor should exit.
    fn handle_key(&mut self, key: KeyEvent) -> Result<bool, String> {
        if key.kind != KeyEventKind::Press {
            return Ok(false);
        }

        // Ctrl+C leaves at once, whatever is open: no save, no copy, no confirm.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.cancelled = true;
            return Ok(true);
        }

        if self.confirm_quit {
            self.confirm_quit = false;
            match key.code {
                KeyCode::Char('s') | KeyCode::Char('y') | KeyCode::Enter => {
                    self.save()?;
                    return Ok(true);
                }
                KeyCode::Char('d') | KeyCode::Char('n') => return Ok(true),
                _ => return Ok(false),
            }
        }

        match self.mode {
            Mode::Form => self.handle_form(key),
            Mode::EditList => {
                self.handle_edit_list(key);
                Ok(false)
            }
            Mode::EditText(_) => {
                self.handle_edit_text(key);
                Ok(false)
            }
        }
    }

    fn handle_form(&mut self, key: KeyEvent) -> Result<bool, String> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.is_generator() {
                    // Nothing is saved in the generator, so there is nothing to confirm.
                    return Ok(true);
                }
                if self.dirty {
                    self.confirm_quit = true;
                    return Ok(false);
                }
                return Ok(true);
            }
            KeyCode::Char('s') => {
                if self.is_generator() {
                    self.copy_now();
                } else {
                    self.save()?;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Char('i') if FIELDS[self.selected_field()].key == "wayland_monitor" => {
                self.refresh_active_monitor();
                self.status = self.monitor_summary();
            }
            // In the generator the line can name the binary by path, which is what gaming mode needs.
            KeyCode::Char('p') if self.is_generator() => {
                self.absolute = !self.absolute;
                self.status = format!("Program: {}", self.program());
            }
            KeyCode::PageUp => self.move_selection(-5),
            KeyCode::PageDown => self.move_selection(5),
            KeyCode::Home => self.jump_to_first(),
            KeyCode::End => self.jump_to_last(),
            KeyCode::Enter | KeyCode::Char(' ') => self.activate(),
            KeyCode::Left => self.cycle_choice(-1),
            KeyCode::Right => self.cycle_choice(1),
            _ => {}
        }
        Ok(false)
    }

    /// Move a choice setting to its previous or next allowed value.
    fn cycle_choice(&mut self, delta: i64) {
        let field = &FIELDS[self.selected_field()];
        if field.key == "wayland_monitor" {
            self.step_monitor(delta);
            return;
        }
        let choices = choices_for(field.key);
        if choices.is_empty() {
            return;
        }
        let current = self.get_integer(field.key);
        let index = choices.iter().position(|c| *c == current).unwrap_or(0) as i64;
        let len = choices.len() as i64;
        let next = (index + delta).rem_euclid(len) as usize;
        self.set_value(field.key, Value::from(choices[next]));
    }

    fn handle_edit_text(&mut self, key: KeyEvent) {
        let Mode::EditText(edit) = &mut self.mode else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.cancel_edit(),
            KeyCode::Enter => self.commit_edit(),
            KeyCode::Backspace => {
                if edit.cursor > 0 {
                    edit.cursor -= 1;
                    let byte = char_byte_index(&edit.buffer, edit.cursor);
                    edit.buffer.remove(byte);
                }
            }
            KeyCode::Delete => {
                if edit.cursor < edit.buffer.chars().count() {
                    let byte = char_byte_index(&edit.buffer, edit.cursor);
                    edit.buffer.remove(byte);
                }
            }
            KeyCode::Left => edit.cursor = edit.cursor.saturating_sub(1),
            KeyCode::Right => {
                edit.cursor = (edit.cursor + 1).min(edit.buffer.chars().count());
            }
            KeyCode::Home => edit.cursor = 0,
            KeyCode::End => edit.cursor = edit.buffer.chars().count(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let byte = char_byte_index(&edit.buffer, edit.cursor);
                edit.buffer.insert(byte, c);
                edit.cursor += 1;
            }
            _ => {}
        }
    }

    /// Leave the text editor without writing anything.
    ///
    /// A list entry goes back to the list, which is where the user came from.
    fn cancel_edit(&mut self) {
        let Mode::EditText(edit) = &self.mode else {
            return;
        };
        let entry = edit.entry;
        self.mode = if entry.is_some() {
            Mode::EditList
        } else {
            Mode::Form
        };
    }

    /// Write the text buffer back into the document (or the edited list entry).
    fn commit_edit(&mut self) {
        let Mode::EditText(edit) = &self.mode else {
            return;
        };
        let key = edit.key.clone();
        let buffer = edit.buffer.clone();
        let entry = edit.entry;
        let adding = edit.adding;

        if let Some(index) = entry {
            let mut items = self.get_list(&key);
            if adding {
                let at = index.min(items.len());
                if buffer.trim().is_empty() {
                    self.status = format!("{key}: empty entry not added");
                } else {
                    items.insert(at, buffer);
                    self.set_list(&key, &items);
                    self.list_selection = at;
                }
            } else if index < items.len() {
                if buffer.trim().is_empty() {
                    self.status = format!("{key}: empty entry not saved");
                } else {
                    items[index] = buffer;
                    self.set_list(&key, &items);
                    self.list_selection = index;
                }
            }
            // Stay in the list the entry came from instead of jumping back to the form.
            self.mode = Mode::EditList;
            return;
        }

        match self.kind(&key) {
            Kind::Integer => match buffer.trim().parse::<i64>() {
                Ok(n) => {
                    let choices = choices_for(&key);
                    if !choices.is_empty() && !choices.contains(&n) {
                        self.status =
                            format!("{key}: {n} is not one of {choices:?}, keeping the old value");
                    } else if choices.is_empty() && n < 0 {
                        self.status = format!(
                            "{key}: negative values are not allowed, keeping the old value"
                        );
                    } else {
                        self.set_value(&key, Value::from(n));
                    }
                }
                Err(_) => {
                    self.status =
                        format!("{key}: '{buffer}' is not a number, keeping the old value");
                }
            },
            _ => self.set_value(&key, Value::from(buffer)),
        }
        self.mode = Mode::Form;
    }

    fn handle_edit_list(&mut self, key: KeyEvent) {
        let field = &FIELDS[self.selected_field()];
        let field_key = field.key.to_string();
        let items = self.get_list(&field_key);

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Form;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.list_selection = self.list_selection.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.list_selection + 1 < items.len() {
                    self.list_selection += 1;
                }
            }
            KeyCode::Char('a') => {
                // The entry is only inserted when the text is committed, so cancelling adds nothing.
                self.mode = Mode::EditText(TextEdit {
                    key: field_key,
                    entry: Some(items.len()),
                    adding: true,
                    buffer: String::new(),
                    cursor: 0,
                });
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(value) = items.get(self.list_selection) {
                    self.mode = Mode::EditText(TextEdit {
                        key: field_key,
                        entry: Some(self.list_selection),
                        adding: false,
                        buffer: value.clone(),
                        cursor: value.chars().count(),
                    });
                }
            }
            KeyCode::Char('d') | KeyCode::Delete if self.list_selection < items.len() => {
                let mut items = items;
                items.remove(self.list_selection);
                self.set_list(&field_key, &items);
                self.list_selection = self.list_selection.min(items.len().saturating_sub(1));
            }
            _ => {}
        }
    }

    // ---- rendering ----

    fn render(&mut self, frame: &mut Frame) {
        // The generator shows one more line: the launch options line itself.
        let footer = if self.is_generator() { 7 } else { 5 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(footer),
            ])
            .split(frame.area());

        self.render_header(frame, chunks[0]);
        if matches!(self.mode, Mode::EditList) {
            self.render_list_editor(frame, chunks[1]);
        } else {
            self.render_form(frame, chunks[1]);
        }
        self.render_footer(frame, chunks[2]);

        if self.confirm_quit {
            self.render_confirm(frame);
        }
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let (title, second) = if self.is_generator() {
            (
                "game-launcher launch options",
                format!("{} (read only, nothing is saved)", self.path.display()),
            )
        } else {
            let modified = if self.dirty { " (modified)" } else { "" };
            (
                "game-launcher config",
                format!("{}{modified}", self.path.display()),
            )
        };
        let text = Line::from(Span::styled(title, Style::new().bold()));
        let path = Line::from(second).dim();
        let block = Paragraph::new(vec![text, path]).block(Block::default().borders(Borders::ALL));
        frame.render_widget(block, area);
    }

    fn render_form(&mut self, frame: &mut Frame, area: Rect) {
        self.normalize_selection();
        let entries = self.entries();
        let items: Vec<ListItem> = entries
            .iter()
            .map(|entry| match entry {
                Entry::Section(title) => ListItem::new(Line::from(format!("-- {title} --")).bold()),
                Entry::Field(index) => {
                    let field = &FIELDS[*index];
                    let flag = if field.flag.is_empty() {
                        "   ".to_string()
                    } else {
                        format!("{:>3}", field.flag)
                    };
                    let mut spans = vec![
                        Span::raw(format!("{:<22}", field.label)),
                        Span::styled(flag, Style::new().dim()),
                        Span::raw("  "),
                        self.value_span(field.key),
                    ];
                    // Read only hint: which monitor this editor is running on.
                    if field.key == "wayland_monitor" {
                        if let Some(note) = self.terminal_note() {
                            spans.push(Span::raw("   "));
                            spans.push(Span::styled(note, Style::new().dim()));
                        }
                    }
                    ListItem::new(Line::from(spans))
                }
            })
            .collect();

        let mut state = ListState::default();
        state.select(Some(self.selection));
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Settings"))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, area, &mut state);

        if let Mode::EditText(edit) = &self.mode {
            self.render_text_edit(frame, area, edit);
        }
    }

    /// Colored preview of a setting's value.
    fn value_span(&self, key: &str) -> Span<'static> {
        match self.kind(key) {
            Kind::Bool => {
                if self.get_bool(key) {
                    Span::styled("on", Style::new().fg(Color::Green))
                } else {
                    Span::styled("off", Style::new().fg(Color::Red))
                }
            }
            Kind::Integer => Span::styled(
                self.get_integer(key).to_string(),
                Style::new().fg(Color::Cyan),
            ),
            Kind::List => {
                let items = self.get_list(key);
                Span::styled(
                    format!("[{} items]", items.len()),
                    Style::new().fg(Color::Magenta),
                )
            }
            Kind::Text if key == "wayland_monitor" => {
                let monitor = self.get_text(key);
                if monitor.is_empty() {
                    Span::styled("(unset)", Style::new().dim())
                } else {
                    Span::styled(monitor, Style::new().fg(Color::Cyan))
                }
            }
            Kind::Text => {
                let text = self.get_text(key);
                if text.is_empty() {
                    Span::styled("(empty)", Style::new().dim())
                } else {
                    Span::styled(text, Style::new().fg(Color::Cyan))
                }
            }
        }
    }

    fn render_text_edit(&self, frame: &mut Frame, area: Rect, edit: &TextEdit) {
        let width = area.width.saturating_sub(4) as usize;
        let popup = centered_rect(area, width.clamp(20, 100) as u16 + 4, 3);
        let prefix: String = edit.buffer.chars().take(edit.cursor).collect();
        let suffix: String = edit.buffer.chars().skip(edit.cursor).collect();
        let line = Line::from(vec![
            Span::styled(&prefix, Style::new().fg(Color::Cyan)),
            Span::styled(&suffix, Style::new().fg(Color::Cyan)),
        ]);
        let block = Block::default().borders(Borders::ALL).title("Editing");
        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new(line).block(block), popup);

        let cursor_x = popup.x + 1 + prefix.chars().count().min(u16::MAX as usize) as u16;
        let cursor_y = popup.y + 1;
        frame.set_cursor_position((cursor_x, cursor_y));
    }

    fn render_list_editor(&self, frame: &mut Frame, area: Rect) {
        let field = &FIELDS[self.selected_field()];
        let key = field.key.to_string();
        let items = self.get_list(&key);
        let entries: Vec<ListItem> = items
            .iter()
            .map(|item| {
                if item.is_empty() {
                    let example = list_example(&key);
                    let text = if example.is_empty() {
                        "(new entry)".to_string()
                    } else {
                        format!("(example: {example})")
                    };
                    ListItem::new(Line::from(text).dim())
                } else {
                    ListItem::new(Line::from(item.clone()))
                }
            })
            .collect();

        let mut state = ListState::default();
        if !items.is_empty() {
            state.select(Some(self.list_selection.min(items.len() - 1)));
        }
        let list = List::new(entries)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("{} ({key})", field.label)),
            )
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, area, &mut state);

        if let Mode::EditText(edit) = &self.mode {
            self.render_text_edit(frame, area, edit);
        }
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let field = &FIELDS[self.selected_field()];
        let keys = match (&self.mode, self.is_generator()) {
            (Mode::EditText(_), _) => {
                "enter commit   esc cancel   <-/-> move   backspace delete   ctrl+c quit without saving"
            }
            (Mode::EditList, _) => {
                "up/down select   enter edit   a add   d delete   esc back   ctrl+c quit without saving"
            }
            (Mode::Form, true) => {
                "up/down select   space toggle/cycle   enter edit   p program   s copy now   q copy and quit   ctrl+c quit without saving"
            }
            (Mode::Form, false) => {
                "up/down select   space toggle/cycle   enter edit   s save   q quit   ctrl+c quit without saving"
            }
        };
        let help = if field.key == "wayland_monitor" {
            let mut text = format!("{}. Press i to list the monitors.", field.help);
            if self.monitors.is_empty() {
                text.push_str(" No monitors detected.");
            }
            text
        } else if let (Kind::List, example) = (self.kind(field.key), list_example(field.key)) {
            if example.is_empty() {
                field.help.to_string()
            } else {
                format!("{} (e.g. {example})", field.help)
            }
        } else {
            field.help.to_string()
        };
        let mut lines = Vec::new();
        if self.is_generator() {
            lines.push(Line::from(vec![
                Span::raw("Launch options: "),
                Span::styled(self.generated_line(), Style::new().fg(Color::Cyan)),
            ]));
        }
        lines.push(Line::from(help));
        lines.push(Line::from(keys).dim());
        lines.push(Line::from(self.status.clone()));
        if self.is_generator() {
            // Changes a flag cannot express (config overrides, removed list entries).
            let (_, notes) = self.launch_flags();
            lines.push(Line::from(notes.join("; ")).dim());
        }
        let block = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        frame.render_widget(block, area);
    }

    fn render_confirm(&self, frame: &mut Frame) {
        let area = centered_rect(frame.area(), 56, 5);
        let text = Paragraph::new(vec![
            Line::from("Unsaved changes").bold(),
            Line::from("s save and quit    d discard and quit    esc cancel"),
        ])
        .block(Block::default().borders(Borders::ALL).title("Quit"));
        frame.render_widget(Clear, area);
        frame.render_widget(text, area);
    }

    /// Draw, read keys, repeat. Returns the launch options line to copy, when there is one.
    fn run_loop(mut self, mut terminal: DefaultTerminal) -> Result<Option<String>, String> {
        loop {
            terminal
                .draw(|frame| self.render(frame))
                .map_err(|e| e.to_string())?;
            let event = event::read().map_err(|e| e.to_string())?;
            if let Event::Key(key) = event {
                if self.handle_key(key)? {
                    if self.is_generator() && !self.cancelled {
                        return Ok(Some(self.generated_line()));
                    }
                    return Ok(None);
                }
            }
        }
    }
}

/// Byte index of character `index` in `text` (or its length when past the end).
fn char_byte_index(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

/// A rectangle of `width` x `height` centered in `area`.
fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Fill in keys that are missing from `doc` using the defaults, so the editor always shows (and saves) the full set of settings.
fn complete_document(doc: &mut DocumentMut, defaults: &DocumentMut) {
    for field in FIELDS {
        if doc.get(field.key).is_some() {
            continue;
        }
        let Some((source_key, source_item)) = defaults.as_table().get_key_value(field.key) else {
            continue;
        };
        let decor = source_key.leaf_decor().clone();
        doc.as_table_mut().insert(field.key, source_item.clone());
        if let Some(mut key) = doc.as_table_mut().key_mut(field.key) {
            *key.leaf_decor_mut() = decor;
        }
    }
}

/// Load the config file and the detection state both editors need.
fn open_editor(name: &str, generator: bool) -> Result<Editor, String> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(format!("the {name} needs an interactive terminal"));
    }
    let Some(path) = config_file::config_path() else {
        return Err("could not resolve a config directory".to_string());
    };

    let (mut doc, warning) = config_file::load_document(&path);
    let defaults = config_file::default_document();
    complete_document(&mut doc, &defaults);

    // Detection runs once; the force flags from the document are applied on top of the GPU result when the row is rendered.
    let monitors = crate::wrappers::detect_monitors();
    let active_monitor = crate::wrappers::active_monitor();
    let mut probe = crate::config::App::default();
    crate::wrappers::determine_wayland_by_gpu(&mut probe);

    let mut editor = if generator {
        Editor::new_generator(path, doc, monitors, active_monitor, probe.wayland_enabled)
    } else {
        Editor::new(path, doc, monitors, active_monitor, probe.wayland_enabled)
    };
    editor.jump_to_first();
    if let Some(warning) = warning {
        editor.status = warning;
    }
    Ok(editor)
}

/// Open the config editor. Returns an error message for the caller to print.
pub fn run() -> Result<(), String> {
    let editor = open_editor("config editor", false)?;
    let terminal = ratatui::init();
    let result = editor.run_loop(terminal);
    ratatui::restore();
    result.map(|_| ())
}

/// Open the launch options generator (`-k`). The config file is only read, never written.
pub fn run_generator() -> Result<(), String> {
    let editor = open_editor("launch options generator", true)?;
    let terminal = ratatui::init();
    let result = editor.run_loop(terminal);
    ratatui::restore();
    if let Some(line) = result? {
        report_generated(&line);
    }
    Ok(())
}

/// Print the generated line and put it on the clipboard, so a missing clipboard tool does not lose it.
fn report_generated(line: &str) {
    println!("{line}");
    match copy_to_clipboard(line) {
        Ok(tool) => println!("Copied to the clipboard with {tool}."),
        Err(e) => println!("Could not reach a clipboard ({e}); the line is printed above."),
    }
}

/// Put `text` on the system clipboard, trying the usual Wayland, X11 and macOS tools.
fn copy_to_clipboard(text: &str) -> Result<&'static str, String> {
    const TOOLS: [(&str, &[&str]); 4] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
        ("pbcopy", &[]),
    ];
    let mut last = "no clipboard tool found".to_string();
    for (tool, args) in TOOLS {
        match write_to_tool(tool, args, text) {
            Ok(()) => return Ok(tool),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Run one clipboard tool with `text` on its stdin.
fn write_to_tool(tool: &str, args: &[&str], text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(tool)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{tool}: {e}"))?;
    {
        let Some(stdin) = child.stdin.as_mut() else {
            return Err(format!("{tool}: no stdin"));
        };
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("{tool}: {e}"))?;
    }
    let _ = child.stdin.take();
    let status = child.wait().map_err(|e| format!("{tool}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{tool} exited with {status}"))
    }
}

/// Quote a value for the generated line when the shell would split it.
fn shell_quote(value: &str) -> String {
    let plain = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/:+=,@%".contains(c));
    if plain {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shorthand for monitor fixtures.
    fn test_monitors(names: &[&str]) -> Vec<Monitor> {
        names
            .iter()
            .map(|name| Monitor {
                name: name.to_string(),
                detail: String::new(),
            })
            .collect()
    }

    #[test]
    fn field_keys_match_default_config() {
        let doc = config_file::default_document();
        let keys: Vec<&str> = doc.iter().map(|(key, _)| key).collect();

        let mut field_keys: Vec<&str> = FIELDS.iter().map(|f| f.key).collect();
        field_keys.sort_unstable();
        let mut expected: Vec<&str> = keys.clone();
        expected.sort_unstable();

        assert_eq!(
            field_keys, expected,
            "the -C editor and DEFAULT_CONFIG drifted apart"
        );
    }

    #[test]
    fn complete_document_adds_every_missing_key() {
        let defaults = config_file::default_document();
        let mut doc = "mangohud = false\n".parse::<DocumentMut>().unwrap();
        complete_document(&mut doc, &defaults);

        for field in FIELDS {
            assert!(doc.get(field.key).is_some(), "{} missing", field.key);
        }
        assert_eq!(
            doc["mangohud"].as_bool(),
            Some(false),
            "existing values win"
        );
        assert_eq!(doc["gamemode"].as_bool(), Some(true));

        let rendered = doc.to_string();
        assert!(
            rendered.contains("# GameMode. -g disables it."),
            "template comments are carried over: {rendered}"
        );
        assert!(
            rendered.contains("# Primary monitor for the Wine Wayland driver"),
            "comments for keys appended at the end are carried over too"
        );
    }

    #[test]
    fn toggle_and_save_round_trip() {
        let dir = std::env::temp_dir().join(format!("game_tui_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        let (mut doc, warning) = config_file::load_document(&path);
        assert!(warning.is_none());
        let defaults = config_file::default_document();
        complete_document(&mut doc, &defaults);

        let mut editor = Editor::new(path.clone(), doc, Vec::new(), None, false);
        // Select MangoHud, a boolean, and toggle it.
        editor.select_key("mangohud");
        editor.activate();
        assert!(!editor.get_bool("mangohud"));
        assert!(editor.dirty);

        editor.save().unwrap();
        let cfg: config_file::FileConfig =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.mangohud, Some(false));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn integer_edit_rejects_garbage() {
        let dir = std::env::temp_dir().join(format!("game_tui_int_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        let doc = config_file::default_document();
        let mut editor = Editor::new(path, doc, Vec::new(), None, false);
        editor.mode = Mode::EditText(TextEdit {
            key: "logging_level".to_string(),
            entry: None,
            adding: false,
            buffer: "nope".to_string(),
            cursor: 4,
        });
        editor.commit_edit();

        assert_eq!(editor.get_integer("logging_level"), 0);
        assert!(editor.status.contains("not a number"));
        assert!(!editor.dirty);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn choice_setting_cycles_only_through_allowed_values() {
        let dir = std::env::temp_dir().join(format!("game_tui_choice_{}", std::process::id()));
        let doc = config_file::default_document();
        let mut editor = Editor::new(dir.join("config.toml"), doc, Vec::new(), None, false);
        editor.select_key("logging_level");

        assert_eq!(editor.get_integer("logging_level"), 0);
        editor.cycle_choice(1);
        assert_eq!(editor.get_integer("logging_level"), 1);
        editor.cycle_choice(1);
        assert_eq!(editor.get_integer("logging_level"), -1, "wraps around");
        editor.cycle_choice(-1);
        assert_eq!(editor.get_integer("logging_level"), 1);
    }

    #[test]
    fn instances_is_edited_as_a_number_and_rejects_negatives() {
        let dir = std::env::temp_dir().join(format!("game_tui_inst_{}", std::process::id()));
        let doc = config_file::default_document();
        let mut editor = Editor::new(dir.join("config.toml"), doc, Vec::new(), None, false);
        editor.select_key("instances");

        // Space on a non-choice integer opens the text editor.
        editor.activate();
        assert!(matches!(editor.mode, Mode::EditText(_)));

        editor.mode = Mode::EditText(TextEdit {
            key: "instances".to_string(),
            entry: None,
            adding: false,
            buffer: "-2".to_string(),
            cursor: 2,
        });
        editor.commit_edit();

        assert_eq!(editor.get_integer("instances"), 1, "kept the old value");
        assert!(editor.status.contains("negative"));
        assert!(!editor.dirty);
    }

    #[test]
    fn wayland_monitor_row_is_only_shown_with_wayland() {
        let dir = std::env::temp_dir().join(format!("game_tui_wl_{}", std::process::id()));
        let doc = config_file::default_document();
        let mut editor = Editor::new(
            dir.join("config.toml"),
            doc,
            test_monitors(&["DP-1"]),
            None,
            false,
        );
        assert!(!editor.visible_keys().contains(&"wayland_monitor"));

        // Forcing Wayland shows it, and forcing it off hides it again, even
        // when the GPU would enable Wayland. Force-enable wins over
        // force-disable, so it is cleared first.
        editor.set_value("wayland_force_enable", Value::from(true));
        assert!(editor.visible_keys().contains(&"wayland_monitor"));
        editor.set_value("wayland_force_enable", Value::from(false));

        editor.gpu_wayland = true;
        editor.set_value("wayland_force_disable", Value::from(true));
        assert!(!editor.visible_keys().contains(&"wayland_monitor"));

        // Otherwise the GPU detection decides.
        editor.set_value("wayland_force_enable", Value::from(false));
        editor.set_value("wayland_force_disable", Value::from(false));
        editor.gpu_wayland = false;
        assert!(!editor.visible_keys().contains(&"wayland_monitor"));
        editor.gpu_wayland = true;
        assert!(editor.visible_keys().contains(&"wayland_monitor"));
    }

    #[test]
    fn wayland_monitor_cycles_detected_outputs_only() {
        let dir = std::env::temp_dir().join(format!("game_tui_wlm_{}", std::process::id()));
        let doc = config_file::default_document();
        let monitors = test_monitors(&["DP-1", "DP-2"]);
        let mut editor = Editor::new(dir.join("config.toml"), doc, monitors, None, true);
        editor.select_key("wayland_monitor");

        assert_eq!(editor.get_text("wayland_monitor"), "");
        editor.activate();
        assert_eq!(editor.get_text("wayland_monitor"), "DP-1");
        assert!(matches!(editor.mode, Mode::Form), "no free text entry");

        editor.activate();
        assert_eq!(editor.get_text("wayland_monitor"), "DP-2");
        editor.activate();
        assert_eq!(
            editor.get_text("wayland_monitor"),
            "",
            "wraps back to unset"
        );

        editor.cycle_choice(-1);
        assert_eq!(
            editor.get_text("wayland_monitor"),
            "DP-2",
            "left arrow steps back"
        );
    }

    #[test]
    fn wayland_monitor_keeps_a_configured_but_undetected_value() {
        let dir = std::env::temp_dir().join(format!("game_tui_wlk_{}", std::process::id()));
        let doc = config_file::default_document();
        let mut editor = Editor::new(
            dir.join("config.toml"),
            doc,
            test_monitors(&["DP-1"]),
            None,
            true,
        );
        editor.set_value("wayland_monitor", Value::from("HDMI-A-1"));

        let choices = editor.monitor_choices();
        assert!(choices.contains(&"HDMI-A-1".to_string()));
    }

    #[test]
    fn wayland_monitor_reports_when_nothing_is_detected() {
        let dir = std::env::temp_dir().join(format!("game_tui_wln_{}", std::process::id()));
        let doc = config_file::default_document();
        let mut editor = Editor::new(dir.join("config.toml"), doc, Vec::new(), None, true);
        editor.select_key("wayland_monitor");
        editor.activate();

        assert_eq!(editor.get_text("wayland_monitor"), "");
        assert!(editor.status.contains("No Wayland monitors"));
        assert!(!editor.dirty);
    }

    #[test]
    fn terminal_note_names_the_current_monitor() {
        let dir = std::env::temp_dir().join(format!("game_tui_wlt_{}", std::process::id()));
        let doc = config_file::default_document();
        let mut editor = Editor::new(
            dir.join("config.toml"),
            doc,
            test_monitors(&["DP-1", "DP-2"]),
            Some("DP-2".to_string()),
            true,
        );

        assert_eq!(
            editor.terminal_note(),
            Some("this terminal: DP-2".to_string())
        );
        editor.active_monitor = None;
        assert_eq!(editor.terminal_note(), None);
    }

    #[test]
    fn monitor_summary_lists_every_output_and_marks_this_terminal() {
        let dir = std::env::temp_dir().join(format!("game_tui_wls_{}", std::process::id()));
        let doc = config_file::default_document();
        let mut monitors = test_monitors(&["DP-2", "DP-1"]);
        monitors[0].detail = "1920x1080 at 0,0".to_string();
        let mut editor = Editor::new(
            dir.join("config.toml"),
            doc,
            monitors,
            Some("DP-1".to_string()),
            true,
        );

        let summary = editor.monitor_summary();
        assert!(summary.contains("DP-2 (1920x1080 at 0,0)"), "{summary}");
        assert!(summary.contains("DP-1 [this terminal]"), "{summary}");

        // The i key refreshes and shows the list in the status line.
        editor.select_key("wayland_monitor");
        assert!(editor
            .handle_form(KeyEvent::from(KeyCode::Char('i')))
            .is_ok());
        assert!(
            editor.status.starts_with("Monitors: "),
            "unexpected status: {}",
            editor.status
        );

        editor.monitors.clear();
        assert_eq!(editor.monitor_summary(), "No monitors detected");
    }

    /// A generator over the default config, ready to toggle rows on.
    fn generator(name: &str) -> Editor {
        let dir = std::env::temp_dir().join(format!("game_tui_gen_{name}_{}", std::process::id()));
        Editor::new_generator(
            dir.join("config.toml"),
            config_file::default_document(),
            Vec::new(),
            None,
            false,
        )
    }

    /// Type into the open text editor and commit it.
    fn type_and_commit(editor: &mut Editor, text: &str) {
        if let Mode::EditText(edit) = &mut editor.mode {
            edit.buffer = text.to_string();
            edit.cursor = text.chars().count();
        }
        editor.handle_edit_text(KeyEvent::from(KeyCode::Enter));
    }

    #[test]
    fn adding_a_list_entry_stays_in_the_list() {
        let dir = std::env::temp_dir().join(format!("game_tui_list_{}", std::process::id()));
        let mut editor = Editor::new(
            dir.join("config.toml"),
            config_file::default_document(),
            Vec::new(),
            None,
            false,
        );
        editor.select_key("mods");
        editor.mode = Mode::EditList;

        editor.handle_edit_list(KeyEvent::from(KeyCode::Char('a')));
        assert!(matches!(editor.mode, Mode::EditText(_)), "the prompt opens");
        type_and_commit(&mut editor, "./mod.sh");

        assert!(matches!(editor.mode, Mode::EditList), "still in the list");
        assert_eq!(editor.get_list("mods"), vec!["./mod.sh".to_string()]);
        assert_eq!(editor.list_selection, 0, "the new entry is selected");
    }

    #[test]
    fn cancelling_a_new_list_entry_adds_nothing() {
        let dir = std::env::temp_dir().join(format!("game_tui_cancel_{}", std::process::id()));
        let mut editor = Editor::new(
            dir.join("config.toml"),
            config_file::default_document(),
            Vec::new(),
            None,
            false,
        );
        editor.select_key("mods");
        editor.mode = Mode::EditList;

        // Committing nothing is refused, and the list keeps its shape.
        editor.handle_edit_list(KeyEvent::from(KeyCode::Char('a')));
        type_and_commit(&mut editor, "   ");
        assert!(matches!(editor.mode, Mode::EditList));
        assert!(editor.get_list("mods").is_empty());
        assert!(editor.status.contains("empty entry not added"));

        // Escaping a typed entry leaves nothing behind either.
        editor.handle_edit_list(KeyEvent::from(KeyCode::Char('a')));
        if let Mode::EditText(edit) = &mut editor.mode {
            edit.buffer = "./mod.sh".to_string();
            edit.cursor = 8;
        }
        editor.handle_edit_text(KeyEvent::from(KeyCode::Esc));
        assert!(matches!(editor.mode, Mode::EditList), "still in the list");
        assert!(editor.get_list("mods").is_empty());
        assert!(!editor.dirty, "cancelling does not dirty the document");
    }

    #[test]
    fn editing_an_existing_list_entry_returns_to_the_list() {
        let dir = std::env::temp_dir().join(format!("game_tui_edit_{}", std::process::id()));
        let mut doc = config_file::default_document();
        let mut array = Array::new();
        array.push("a");
        array.push("b");
        doc["mods"] = Item::Value(Value::Array(array));

        let mut editor = Editor::new(dir.join("config.toml"), doc, Vec::new(), None, false);
        editor.select_key("mods");
        editor.mode = Mode::EditList;
        editor.list_selection = 1;

        editor.handle_edit_list(KeyEvent::from(KeyCode::Enter));
        type_and_commit(&mut editor, "c");

        assert!(matches!(editor.mode, Mode::EditList), "still in the list");
        assert_eq!(
            editor.get_list("mods"),
            vec!["a".to_string(), "c".to_string()]
        );
        assert_eq!(editor.list_selection, 1, "the edited entry stays selected");
    }

    #[test]
    fn ctrl_c_leaves_without_saving() {
        let dir = std::env::temp_dir().join(format!("game_tui_ctrlc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        let mut editor = Editor::new(
            path.clone(),
            config_file::default_document(),
            Vec::new(),
            None,
            false,
        );
        editor.select_key("mangohud");
        editor.activate();
        assert!(editor.dirty);
        assert!(editor.handle_key(ctrl_c).unwrap(), "ctrl+c exits at once");
        assert!(!path.exists(), "nothing was written");

        // It works while typing too, which is where the usual Ctrl+C reflex lands.
        editor.select_key("instances");
        editor.activate();
        assert!(matches!(editor.mode, Mode::EditText(_)));
        assert!(editor.handle_key(ctrl_c).unwrap());
        assert!(editor.cancelled);
        assert!(!path.exists(), "still nothing written");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generator_starts_with_a_bare_line() {
        let editor = generator("bare");
        assert_eq!(editor.generated_line(), "game -- %command%");
        assert!(editor.launch_flags().0.is_empty());
    }

    #[test]
    fn generator_emits_only_the_flags_that_changed() {
        let mut editor = generator("flags");

        editor.select_key("gamemode");
        editor.activate();
        assert_eq!(editor.generated_line(), "game -g -- %command%");

        editor.select_key("gamescope");
        editor.activate();
        assert_eq!(editor.generated_line(), "game -g -s -- %command%");

        editor.select_key("logging_level");
        editor.cycle_choice(1);
        assert_eq!(editor.generated_line(), "game -g -s -l 1 -- %command%");

        // Back to the config value: the flag disappears again.
        editor.select_key("gamescope");
        editor.activate();
        assert_eq!(editor.generated_line(), "game -g -l 1 -- %command%");
    }

    #[test]
    fn generator_reports_changes_that_need_the_config() {
        let dir = std::env::temp_dir().join(format!("game_tui_gennote_{}", std::process::id()));
        let mut doc = config_file::default_document();
        doc["mangohud"] = Item::Value(Value::from(false));
        let mut editor =
            Editor::new_generator(dir.join("config.toml"), doc, Vec::new(), None, false);

        // Turning MangoHud back on cannot be done with a flag, only with the config.
        editor.select_key("mangohud");
        editor.activate();
        let (flags, notes) = editor.launch_flags();
        assert!(flags.is_empty());
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("use -C"), "{}", notes[0]);
    }

    #[test]
    fn generator_adds_exports_and_dll_overrides() {
        let mut editor = generator("lists");
        editor.select_key("exports");
        editor.mode = Mode::EditList;
        editor.handle_edit_list(KeyEvent::from(KeyCode::Char('a')));
        type_and_commit(&mut editor, "PROTON_NO_ESYNC=1");
        assert_eq!(
            editor.generated_line(),
            "game PROTON_NO_ESYNC=1 -- %command%"
        );

        editor.select_key("dll_overrides");
        editor.mode = Mode::EditList;
        editor.handle_edit_list(KeyEvent::from(KeyCode::Char('a')));
        type_and_commit(&mut editor, "dinput8=n,b");
        assert!(
            editor.generated_line().contains("-d dinput8=n,b"),
            "{}",
            editor.generated_line()
        );

        // Values the shell would split are quoted.
        editor.select_key("mods");
        editor.mode = Mode::EditList;
        editor.handle_edit_list(KeyEvent::from(KeyCode::Char('a')));
        type_and_commit(&mut editor, "./my mod.sh --flag");
        assert!(
            editor.generated_line().contains("-u './my mod.sh --flag'"),
            "{}",
            editor.generated_line()
        );
    }

    #[test]
    fn generator_can_name_the_binary_by_path() {
        let mut editor = generator("path");
        editor.binary = "/usr/local/bin/game".to_string();

        editor
            .handle_form(KeyEvent::from(KeyCode::Char('p')))
            .unwrap();
        assert_eq!(editor.generated_line(), "/usr/local/bin/game -- %command%");
        assert!(editor.status.contains("/usr/local/bin/game"));

        editor
            .handle_form(KeyEvent::from(KeyCode::Char('p')))
            .unwrap();
        assert_eq!(editor.generated_line(), "game -- %command%");
    }

    #[test]
    fn generator_quits_without_the_dirty_prompt() {
        let mut editor = generator("quit");
        editor.select_key("gamescope");
        editor.activate();
        assert!(editor.dirty);

        assert!(editor
            .handle_form(KeyEvent::from(KeyCode::Char('q')))
            .unwrap());
        assert!(!editor.confirm_quit, "the generator has nothing to save");
    }

    #[test]
    fn ctrl_c_leaves_the_generator_without_a_line() {
        let mut editor = generator("cancel");
        editor.select_key("gamescope");
        editor.activate();

        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(editor.handle_key(ctrl_c).unwrap());
        assert!(editor.cancelled, "nothing is copied on the way out");
    }
}
