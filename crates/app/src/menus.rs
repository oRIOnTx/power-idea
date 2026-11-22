//! The macOS menu bar, JetBrains style: Power IDEA, File, Edit, View,
//! Navigate, Code, Git, Window, Help. Items dispatch the same actions as the
//! keymap, so shortcuts show up next to them.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input;
use gpui_kit::*;

use crate::actions::*;

pub fn install(cx: &mut App) {
    cx.on_action(|_: &About, cx| show_about(cx));
    cx.on_action(|_: &NewProject, cx| new_project(cx));
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOtherApps, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAllApps, cx| cx.unhide_other_apps());
    cx.on_action(|_: &MinimizeWindow, cx| with_active_window(cx, |window, _| window.minimize_window()));
    cx.on_action(|_: &ZoomWindow, cx| with_active_window(cx, |window, _| window.zoom_window()));
    cx.set_menus(menus());
}

fn with_active_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App)) {
    if let Some(window) = cx.active_window() {
        window.update(cx, |_, window, cx| f(window, cx)).ok();
    }
}

fn show_about(cx: &mut App) {
    with_active_window(cx, |window, cx| {
        window.open_alert_dialog(cx, |dialog, _, _| {
            dialog
                .title("Power IDEA")
                .child(format!("Version {}", crate::APP_VERSION))
        });
    });
}

/// Opens the New Project dialog in the active window (Welcome or a project).
fn new_project(cx: &mut App) {
    if cx.active_window().is_none() {
        crate::welcome::show(cx);
    }
    with_active_window(cx, crate::welcome::open_new_project_dialog);
}

fn menus() -> Vec<Menu> {
    vec![
        Menu::new("Power IDEA").items([
            MenuItem::action("About Power IDEA", About),
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Power IDEA", HideApp),
            MenuItem::action("Hide Others", HideOtherApps),
            MenuItem::action("Show All", ShowAllApps),
            MenuItem::separator(),
            MenuItem::action("Quit Power IDEA", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Project…", NewProject),
            MenuItem::action("Open…", OpenProject),
            MenuItem::action("Recent Projects…", RecentProjects),
            MenuItem::separator(),
            MenuItem::action("Save", SaveFile),
            MenuItem::action("Close Tab", CloseTab),
            MenuItem::action("Close Project", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", input::Undo, OsAction::Undo),
            MenuItem::os_action("Redo", input::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
            MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
            MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("Find in Files…", FindInFiles),
            MenuItem::action("Replace in Files…", ReplaceInFiles),
            MenuItem::action("Replace", input::Replace),
            MenuItem::separator(),
            MenuItem::action("Comment with Line Comment", ToggleLineComment),
            MenuItem::action("Duplicate Line", DuplicateLine),
            MenuItem::action("Delete Line", DeleteLine),
            MenuItem::action("Move Line Up", MoveLineUp),
            MenuItem::action("Move Line Down", MoveLineDown),
        ]),
        Menu::new("View").items([
            MenuItem::action("Project", ToggleProjectView),
            MenuItem::action("Commit", ToggleCommitView),
            MenuItem::action("Git Log", ShowGitLog),
            MenuItem::action("Database", ToggleDatabaseView),
            MenuItem::action("Docker", ToggleDockerView),
            MenuItem::action("Claude Insights", ToggleInsightsView),
            MenuItem::action("Terminal", ToggleTerminal),
            MenuItem::separator(),
            MenuItem::action("Toggle Preview", TogglePreview),
            MenuItem::action("Toggle Light/Dark Theme", ToggleTheme),
        ]),
        Menu::new("Navigate").items([
            MenuItem::action("Back", NavigateBack),
            MenuItem::action("Forward", NavigateForward),
            MenuItem::separator(),
            MenuItem::action("Search Everywhere", SearchEverywhere),
            MenuItem::action("Go to Class…", GotoClass),
            MenuItem::action("Go to File…", GotoFile),
            MenuItem::action("Go to Symbol…", GotoSymbol),
            MenuItem::action("Go to Line…", GotoLine),
            MenuItem::action("Recent Files", RecentFiles),
            MenuItem::separator(),
            MenuItem::action("Go to Definition", input::GoToDefinition),
            MenuItem::action("File Structure", FileStructure),
            MenuItem::action("Find Usages", FindUsages),
        ]),
        Menu::new("Code").items([
            MenuItem::action("Reformat Code", ReformatCode),
            MenuItem::action("Rename…", RenameSymbol),
        ]),
        Menu::new("Run").items([
            MenuItem::action("Run", RunConfiguration),
            MenuItem::action("Run…", ChooseRunConfiguration),
            MenuItem::action("Debug", DebugConfiguration),
            MenuItem::action("Stop", StopRun),
            MenuItem::separator(),
            MenuItem::action("Resume Program", DebugResume),
            MenuItem::action("Pause Program", DebugPause),
            MenuItem::action("Step Over", StepOver),
            MenuItem::action("Step Into", StepInto),
            MenuItem::action("Step Out", StepOut),
            MenuItem::action("Toggle Line Breakpoint", ToggleBreakpoint),
            MenuItem::separator(),
            MenuItem::action("Edit Configurations…", EditRunConfigurations),
        ]),
        Menu::new("Git").items([
            MenuItem::action("Commit…", CommitChanges),
            MenuItem::action("Push…", Push),
            MenuItem::action("Fetch", Fetch),
            MenuItem::action("Update Project…", UpdateProject),
            MenuItem::separator(),
            MenuItem::action("Branches…", Branches),
            MenuItem::action("Show Git Log", ShowGitLog),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", MinimizeWindow),
            MenuItem::action("Zoom", ZoomWindow),
            MenuItem::separator(),
            MenuItem::action("New Terminal Tab", NewTerminal),
        ]),
        Menu::new("Help").items([
            MenuItem::action("Find Action…", FindAction),
            MenuItem::action("Copy Claude MCP Command", CopyMcpCommand),
        ]),
    ]
}
