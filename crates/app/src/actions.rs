//! IDE actions and the default JetBrains (macOS) keymap.
//!
//! Users can override or extend bindings in `keymap.toml` next to `config.toml`:
//!
//! ```toml
//! [[binding]]
//! keys = "cmd-p"
//! action = "ide::GotoFile"
//! context = "Workspace"   # optional
//! ```

use std::fs;
use std::rc::Rc;

use gpui_kit::*;
use serde::Deserialize;

use crate::config::Config;
use crate::github_actions as gha;

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const CODE_EDITOR_CONTEXT: &str = "CodeEditor";
pub const PALETTE_CONTEXT: &str = "Palette";
/// The Branches popup under the title bar's branch widget.
pub const BRANCHES_CONTEXT: &str = "Branches";
pub const TERMINAL_CONTEXT: &str = "Terminal";
/// The Git Log commit list (arrow keys move the selection).
pub const GIT_LOG_CONTEXT: &str = "GitLog";
/// The Find Usages results (arrows select, Enter jumps).
pub const USAGES_CONTEXT: &str = "Usages";
/// The GitHub Actions tab with focus outside its text fields (single-letter keys act there).
const CI_LIST: &str = "CiView && !Input";
/// The project tree (⌘X / ⌘C / ⌘V act on the selected files).
pub const PROJECT_TREE_CONTEXT: &str = "ProjectTree";

actions!(
    ide,
    [
        SearchEverywhere,
        GotoFile,
        RecentFiles,
        FindInFiles,
        FindAction,
        GotoLine,
        SaveFile,
        CloseTab,
        NextTab,
        PrevTab,
        ToggleProjectView,
        OpenProject,
        RecentProjects,
        ToggleTheme,
        CloseWindow,
        Quit,
        ToggleLineComment,
        DuplicateLine,
        DeleteLine,
        MoveLineUp,
        MoveLineDown,
        ToggleTerminal,
        NewTerminal,
        CloseTerminal,
        ToggleCommitView,
        CommitChanges,
        Push,
        Fetch,
        ResolveConflicts,
        UpdateProject,
        ShowGitLog,
        Branches,
        NewBranch,
        NewWorktree,
        CheckoutRevision,
        CopyMcpCommand,
        SmartEnter,
        SmartBackspace,
        SmartTab,
        CutFiles,
        CopyFiles,
        PasteFiles,
        DeleteFiles,
        OpenSettings,
        NavigateBack,
        NavigateForward,
        ToggleDatabaseView,
        TogglePreview,
        FindUsages,
        GotoClass,
        GotoSymbol,
        FileStructure,
        RenameSymbol,
        ReformatCode,
        About,
        NewProject,
        HideApp,
        HideOtherApps,
        ShowAllApps,
        MinimizeWindow,
        ZoomWindow,
        OpenSelectedUsage,
        ReplaceInFiles,
        RunConfiguration,
        ChooseRunConfiguration,
        StopRun,
        EditRunConfigurations,
        DebugConfiguration,
        ToggleBreakpoint,
        DebugResume,
        DebugPause,
        StepOver,
        StepInto,
        StepOut,
        ToggleDockerView,
        ShowGitHubActions,
        ToggleInsightsView,
        ToggleClaude,
        ClaudeNewSession,
        ClaudeContinueSession,
        ClaudeResumeSession,
        ClaudeRestartSession,
        ClaudeCloseSession,
        ReloadExtensions,
        OpenExtensionsFolder,
        ShowErrors,
        TriggerTestError,
        TriggerTestPanic,
        TriggerTestCrash,
    ]
);

actions!(palette, [SelectNext, SelectPrev]);

/// Operations on a data source in the Database tool window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseOp {
    NewDataSource,
    OpenConsole,
    Edit,
    Refresh,
    Disconnect,
    DiscoverDatabases,
    Remove,
}

/// Runs `op` on the data source `id`, or on its `database` when that is not empty.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = database, no_json)]
pub struct DatabaseAction {
    pub op: DatabaseOp,
    pub id: SharedString,
    pub database: SharedString,
}

impl DatabaseAction {
    pub fn boxed(op: DatabaseOp, id: impl Into<SharedString>) -> Box<dyn Action> {
        Self::boxed_for(op, id, "")
    }

    pub fn boxed_for(
        op: DatabaseOp,
        id: impl Into<SharedString>,
        database: impl Into<SharedString>,
    ) -> Box<dyn Action> {
        Box::new(Self {
            op,
            id: id.into(),
            database: database.into(),
        })
    }
}

/// Operations from the Git Log tool window on a commit, branch or the repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitLogOp {
    CopyRevision,
    NewBranchAt,
    NewTagAt,
    CheckoutRevision,
    CherryPick,
    Revert,
    ResetSoft,
    ResetMixed,
    ResetHard,
    CheckoutBranch,
    MergeIntoCurrent,
    RebaseCurrentOnto,
    RenameBranch,
    DeleteBranch,
    PushBranch,
    NewWorktreeFrom,
    DiffWithWorkingTree,
    SetTrackedBranch,
    /// Pull into the current branch (JetBrains "Update").
    UpdateBranch,
    ContinueOperation,
    AbortOperation,
    Refresh,
}

/// Runs `op` on `target`: a commit hash or a branch name (ignored by repository ops).
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = git, no_json)]
pub struct GitLogAction {
    pub op: GitLogOp,
    pub target: SharedString,
}

impl GitLogAction {
    pub fn boxed(op: GitLogOp, target: impl Into<SharedString>) -> Box<dyn Action> {
        Box::new(Self {
            op,
            target: target.into(),
        })
    }
}

/// File operations from the project tree and tab context menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileOp {
    NewFile,
    NewDirectory,
    Cut,
    CopyFile,
    Paste,
    Rename,
    Delete,
    CopyPath,
    CopyRelativePath,
    RevealInFinder,
    OpenInTerminal,
    ShowDiff,
    CloseTab,
    CloseOtherTabs,
    CloseAllTabs,
}

/// Runs `op` on `path` (relative to the project root; `""` is the root).
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = project, no_json)]
pub struct FileAction {
    pub op: FileOp,
    pub path: SharedString,
}

impl FileAction {
    pub fn boxed(op: FileOp, path: impl Into<SharedString>) -> Box<dyn Action> {
        Box::new(Self {
            op,
            path: path.into(),
        })
    }
}

/// Operations from the terminal tab context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalTabOp {
    Rename,
    Pin,
    Unpin,
    Close,
    CloseOthers,
}

/// Runs `op` on the terminal tab whose view has entity id `tab`.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = terminal, no_json)]
pub struct TerminalTabAction {
    pub op: TerminalTabOp,
    pub tab: u64,
}

/// Runs the extension command `id` (from an `extension.toml`).
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = extensions, no_json)]
pub struct RunExtensionCommand {
    pub id: SharedString,
}

/// Actions listed in "Find Action" (Cmd+Shift+A), in display order.
pub fn palette_actions() -> Vec<(&'static str, Box<dyn Action>)> {
    let mut actions: Vec<(&'static str, Box<dyn Action>)> = vec![
        ("Search Everywhere", Box::new(SearchEverywhere)),
        ("Go to File…", Box::new(GotoFile)),
        ("Recent Files", Box::new(RecentFiles)),
        ("Find in Files…", Box::new(FindInFiles)),
        ("Replace in Files…", Box::new(ReplaceInFiles)),
        ("Run", Box::new(RunConfiguration)),
        ("Run…", Box::new(ChooseRunConfiguration)),
        ("Stop", Box::new(StopRun)),
        ("Edit Run Configurations…", Box::new(EditRunConfigurations)),
        ("Debug", Box::new(DebugConfiguration)),
        ("Toggle Line Breakpoint", Box::new(ToggleBreakpoint)),
        ("Resume Program", Box::new(DebugResume)),
        ("Pause Program", Box::new(DebugPause)),
        ("Step Over", Box::new(StepOver)),
        ("Step Into", Box::new(StepInto)),
        ("Step Out", Box::new(StepOut)),
        ("Go to Line…", Box::new(GotoLine)),
        ("Go to Class…", Box::new(GotoClass)),
        ("Go to Symbol…", Box::new(GotoSymbol)),
        ("File Structure", Box::new(FileStructure)),
        ("Find Usages", Box::new(FindUsages)),
        ("Rename…", Box::new(RenameSymbol)),
        ("Reformat Code", Box::new(ReformatCode)),
        ("Save", Box::new(SaveFile)),
        ("Close Tab", Box::new(CloseTab)),
        ("Next Tab", Box::new(NextTab)),
        ("Previous Tab", Box::new(PrevTab)),
        ("Toggle Project View", Box::new(ToggleProjectView)),
        ("New Project…", Box::new(NewProject)),
        ("Open File or Project…", Box::new(OpenProject)),
        ("Close Project", Box::new(CloseWindow)),
        ("About Power IDEA", Box::new(About)),
        ("Recent Projects…", Box::new(RecentProjects)),
        ("Toggle Light/Dark Theme", Box::new(ToggleTheme)),
        ("Comment Line", Box::new(ToggleLineComment)),
        ("Duplicate Line", Box::new(DuplicateLine)),
        ("Delete Line", Box::new(DeleteLine)),
        ("Move Line Up", Box::new(MoveLineUp)),
        ("Move Line Down", Box::new(MoveLineDown)),
        ("Terminal", Box::new(ToggleTerminal)),
        ("New Terminal Tab", Box::new(NewTerminal)),
        ("Commit…", Box::new(CommitChanges)),
        ("Push…", Box::new(Push)),
        ("Fetch", Box::new(Fetch)),
        ("Resolve Conflicts…", Box::new(ResolveConflicts)),
        ("Update Project (Pull)", Box::new(UpdateProject)),
        ("Git Log", Box::new(ShowGitLog)),
        ("Refresh Git Log", GitLogAction::boxed(GitLogOp::Refresh, "")),
        ("Branches…", Box::new(Branches)),
        ("New Branch…", Box::new(NewBranch)),
        ("New Worktree…", Box::new(NewWorktree)),
        ("Checkout Tag or Revision…", Box::new(CheckoutRevision)),
        ("Toggle Commit View", Box::new(ToggleCommitView)),
        ("Settings…", Box::new(OpenSettings)),
        ("Database", Box::new(ToggleDatabaseView)),
        ("Docker", Box::new(ToggleDockerView)),
        ("GitHub Actions", Box::new(ShowGitHubActions)),
        ("Claude Session Insights", Box::new(ToggleInsightsView)),
        ("Claude Code", Box::new(ToggleClaude)),
        ("Claude: New Session", Box::new(ClaudeNewSession)),
        ("Claude: Continue Last Session", Box::new(ClaudeContinueSession)),
        ("Claude: Resume Session…", Box::new(ClaudeResumeSession)),
        ("Claude: Restart Session", Box::new(ClaudeRestartSession)),
        ("Claude: Close Session", Box::new(ClaudeCloseSession)),
        (
            "Toggle Preview (Editor / Split / Preview)",
            Box::new(TogglePreview),
        ),
        (
            "New Data Source…",
            DatabaseAction::boxed(DatabaseOp::NewDataSource, ""),
        ),
        ("Back", Box::new(NavigateBack)),
        ("Forward", Box::new(NavigateForward)),
        ("Copy Claude MCP Command", Box::new(CopyMcpCommand)),
        ("Reload Extensions", Box::new(ReloadExtensions)),
        ("Open Extensions Folder", Box::new(OpenExtensionsFolder)),
        ("Cut Selected Files", Box::new(CutFiles)),
        ("Copy Selected Files", Box::new(CopyFiles)),
        ("Paste Files", Box::new(PasteFiles)),
        ("Delete Selected Files…", Box::new(DeleteFiles)),
        ("Show Errors", Box::new(ShowErrors)),
        ("Quit", Box::new(Quit)),
    ];
    // Hidden helpers for checking the error badge by hand; never in release builds.
    if cfg!(debug_assertions) {
        actions.extend([
            (
                "Debug: Trigger Test Error",
                Box::new(TriggerTestError) as Box<dyn Action>,
            ),
            (
                "Debug: Trigger Test Panic (background thread)",
                Box::new(TriggerTestPanic),
            ),
            (
                "Debug: Trigger Test Crash (UI thread, aborts)",
                Box::new(TriggerTestCrash),
            ),
        ]);
    }
    actions
}

fn default_keymap() -> Vec<KeyBinding> {
    let ws = Some(WORKSPACE_CONTEXT);
    // Editor bindings must out-rank the generic "Input" bindings of the editor widget.
    let ed = Some("CodeEditor > Input");
    vec![
        KeyBinding::new("shift shift", SearchEverywhere, ws),
        KeyBinding::new("cmd-shift-o", GotoFile, ws),
        KeyBinding::new("cmd-e", RecentFiles, ws),
        KeyBinding::new("cmd-shift-f", FindInFiles, ws),
        KeyBinding::new("cmd-shift-f", FindInFiles, ed),
        KeyBinding::new("cmd-shift-r", ReplaceInFiles, ws),
        // Not in the terminal, where Ctrl-R is the shell's history search.
        KeyBinding::new("ctrl-r", RunConfiguration, Some("Workspace && !Terminal")),
        KeyBinding::new(
            "ctrl-alt-r",
            ChooseRunConfiguration,
            Some("Workspace && !Terminal"),
        ),
        KeyBinding::new("cmd-f2", StopRun, ws),
        KeyBinding::new(
            "ctrl-d",
            DebugConfiguration,
            Some("Workspace && !Terminal && !Input"),
        ),
        KeyBinding::new("cmd-f8", ToggleBreakpoint, ws),
        KeyBinding::new("cmd-f8", ToggleBreakpoint, ed),
        KeyBinding::new("cmd-alt-r", DebugResume, ws),
        KeyBinding::new("f8", StepOver, ws),
        KeyBinding::new("f7", StepInto, ws),
        KeyBinding::new("shift-f8", StepOut, ws),
        KeyBinding::new("cmd-shift-r", ReplaceInFiles, ed),
        // Through the workspace first: closing blocks, a level back in indentation.
        KeyBinding::new("enter", SmartEnter, ed),
        KeyBinding::new("backspace", SmartBackspace, ed),
        // JetBrains: Tab accepts the highlighted completion item.
        KeyBinding::new("tab", SmartTab, ed),
        // JetBrains Basic Completion.
        KeyBinding::new("ctrl-space", component::input::ShowCompletions, ed),
        KeyBinding::new("cmd-shift-a", FindAction, ws),
        KeyBinding::new("cmd-l", GotoLine, ws),
        KeyBinding::new("cmd-s", SaveFile, ws),
        KeyBinding::new("cmd-w", CloseTab, ws),
        KeyBinding::new("ctrl-tab", NextTab, ws),
        KeyBinding::new("cmd-shift-]", NextTab, ws),
        KeyBinding::new("ctrl-shift-tab", PrevTab, ws),
        KeyBinding::new("cmd-shift-[", PrevTab, ws),
        KeyBinding::new("cmd-1", ToggleProjectView, ws),
        KeyBinding::new("cmd-o", GotoClass, ws),
        KeyBinding::new("cmd-alt-o", GotoSymbol, ws),
        KeyBinding::new("cmd-f12", FileStructure, ws),
        KeyBinding::new("alt-f7", FindUsages, ws),
        KeyBinding::new("alt-f7", FindUsages, ed),
        KeyBinding::new("shift-f6", RenameSymbol, ed),
        KeyBinding::new("cmd-alt-l", ReformatCode, ed),
        KeyBinding::new("cmd-shift-w", CloseWindow, ws),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("cmd-alt-h", HideOtherApps, None),
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        KeyBinding::new("cmd-/", ToggleLineComment, ed),
        KeyBinding::new("cmd-b", component::input::GoToDefinition, ed),
        KeyBinding::new(
            "cmd-enter",
            crate::database::console::RunQuery,
            Some("QueryConsole > Input"),
        ),
        KeyBinding::new(
            "cmd-enter",
            crate::database::console::RunQuery,
            Some("QueryConsole"),
        ),
        KeyBinding::new("cmd-[", NavigateBack, ws),
        KeyBinding::new("cmd-[", NavigateBack, ed),
        KeyBinding::new("cmd-]", NavigateForward, ws),
        KeyBinding::new("cmd-]", NavigateForward, ed),
        KeyBinding::new("cmd-alt-left", NavigateBack, ws),
        KeyBinding::new("cmd-alt-right", NavigateForward, ws),
        KeyBinding::new("cmd-d", DuplicateLine, ed),
        KeyBinding::new("cmd-backspace", DeleteLine, ed),
        KeyBinding::new("alt-shift-up", MoveLineUp, ed),
        KeyBinding::new("alt-shift-down", MoveLineDown, ed),
        KeyBinding::new("cmd-r", component::input::Replace, ed),
        KeyBinding::new("alt-f12", ToggleTerminal, ws),
        // As in Claude Code's own JetBrains plugin.
        KeyBinding::new("cmd-escape", ToggleClaude, ws),
        KeyBinding::new("cmd-0", ToggleCommitView, ws),
        KeyBinding::new("cmd-,", OpenSettings, ws),
        KeyBinding::new("cmd-k", CommitChanges, ws),
        KeyBinding::new("cmd-shift-k", Push, ws),
        KeyBinding::new("cmd-t", UpdateProject, ws),
        KeyBinding::new("alt-cmd-n", NewBranch, ws),
        KeyBinding::new("cmd-9", ShowGitLog, ws),
        KeyBinding::new("cmd-8", ShowGitHubActions, ws),
        KeyBinding::new("cmd-t", NewTerminal, Some(TERMINAL_CONTEXT)),
        KeyBinding::new("cmd-w", CloseTerminal, Some(TERMINAL_CONTEXT)),
        KeyBinding::new("cmd-c", crate::terminal::Copy, Some(TERMINAL_CONTEXT)),
        KeyBinding::new("cmd-v", crate::terminal::Paste, Some(TERMINAL_CONTEXT)),
        KeyBinding::new("cmd-k", crate::terminal::Clear, Some(TERMINAL_CONTEXT)),
        KeyBinding::new("cmd-x", CutFiles, Some(PROJECT_TREE_CONTEXT)),
        KeyBinding::new("cmd-c", CopyFiles, Some(PROJECT_TREE_CONTEXT)),
        KeyBinding::new("cmd-v", PasteFiles, Some(PROJECT_TREE_CONTEXT)),
        // JetBrains on macOS: ⌦, ⌫ and ⌘⌫ all delete in the project tree.
        KeyBinding::new("delete", DeleteFiles, Some(PROJECT_TREE_CONTEXT)),
        KeyBinding::new("backspace", DeleteFiles, Some(PROJECT_TREE_CONTEXT)),
        KeyBinding::new("cmd-backspace", DeleteFiles, Some(PROJECT_TREE_CONTEXT)),
        KeyBinding::new("up", SelectPrev, Some(USAGES_CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(USAGES_CONTEXT)),
        KeyBinding::new("enter", OpenSelectedUsage, Some(USAGES_CONTEXT)),
        KeyBinding::new("f4", OpenSelectedUsage, Some(USAGES_CONTEXT)),
        KeyBinding::new("up", SelectPrev, Some(CI_LIST)),
        KeyBinding::new("down", SelectNext, Some(CI_LIST)),
        KeyBinding::new("enter", gha::OpenSelected, Some(CI_LIST)),
        KeyBinding::new("escape", gha::Back, Some(CI_LIST)),
        KeyBinding::new("escape", gha::Back, Some("CiSearch > Input")),
        KeyBinding::new("o", gha::OpenOnGitHub, Some(CI_LIST)),
        KeyBinding::new("cmd-r", gha::Refresh, Some(gha::CI_CONTEXT)),
        KeyBinding::new("cmd-f", gha::FindInLog, Some(gha::CI_CONTEXT)),
        KeyBinding::new("ctrl-f", gha::FindInLog, Some(gha::CI_CONTEXT)),
        KeyBinding::new("cmd-c", gha::CopySelection, Some(CI_LIST)),
        KeyBinding::new("up", SelectPrev, Some(GIT_LOG_CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(GIT_LOG_CONTEXT)),
        KeyBinding::new("up", SelectPrev, Some("Welcome > Input")),
        KeyBinding::new("down", SelectNext, Some("Welcome > Input")),
        KeyBinding::new("up", SelectPrev, Some("Palette > Input")),
        KeyBinding::new("down", SelectNext, Some("Palette > Input")),
        KeyBinding::new("up", SelectPrev, Some("Branches > Input")),
        KeyBinding::new("down", SelectNext, Some("Branches > Input")),
    ]
}

#[derive(Deserialize, Default)]
struct UserKeymap {
    #[serde(default)]
    binding: Vec<UserBinding>,
}

#[derive(Deserialize)]
struct UserBinding {
    keys: String,
    action: String,
    context: Option<String>,
}

/// Loads user overrides; invalid entries are reported and skipped.
fn user_keymap(cx: &App) -> Vec<KeyBinding> {
    let Some(path) = Config::dir().map(|dir| dir.join("keymap.toml")) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let keymap: UserKeymap = match toml::from_str(&text) {
        Ok(keymap) => keymap,
        Err(err) => {
            eprintln!("{}: {err}", path.display());
            return Vec::new();
        }
    };

    keymap
        .binding
        .into_iter()
        .filter_map(|b| {
            let action = cx
                .build_action(&b.action, None)
                .map_err(|err| eprintln!("keymap: unknown action {}: {err}", b.action))
                .ok()?;
            let context = match b.context.as_deref().map(KeyBindingContextPredicate::parse) {
                Some(Ok(predicate)) => Some(Rc::new(predicate)),
                Some(Err(err)) => {
                    eprintln!("keymap: bad context for {}: {err}", b.keys);
                    return None;
                }
                None => None,
            };
            KeyBinding::load(&b.keys, action, context, false, None, cx)
                .map_err(|err| eprintln!("keymap: bad keys {}: {err}", b.keys))
                .ok()
        })
        .collect()
}

/// Bindings registered by gpui-kit's own components before ours, restored on rebinding.
struct ComponentKeymap(Vec<KeyBinding>);

impl Global for ComponentKeymap {}

/// Installs the default keymap, then extension commands' bindings, then the user's
/// `keymap.toml` (later bindings win). Called again when extensions reload.
pub fn bind_keys(cx: &mut App) {
    if !cx.has_global::<ComponentKeymap>() {
        let components = cx.key_bindings().borrow().bindings().cloned().collect();
        cx.set_global(ComponentKeymap(components));
    }
    let components = cx.global::<ComponentKeymap>().0.clone();
    cx.clear_key_bindings();
    cx.bind_keys(components);
    cx.bind_keys(default_keymap());
    let extensions = crate::extension_registry::key_bindings(cx);
    cx.bind_keys(extensions);
    let user = user_keymap(cx);
    cx.bind_keys(user);
}
