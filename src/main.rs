mod assets;
mod config;
mod coords;
mod daemon;
mod display;
mod document;
mod editor;
mod images;
mod mermaid;
mod mode;
mod motion;
mod notion;
mod palette;
mod slash;
mod surface;
mod syntax;
mod tabs;
mod theme;
mod tree;
mod undo;
mod video;
mod wysiwyg;

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use gpui::{
    point, px, size, AnyWindowHandle, App, AppContext as _, BorrowAppContext as _, Entity, Global,
    QuitMode, Styled as _, TitlebarOptions, WeakEntity, WindowBounds, WindowOptions,
};
use gpui_component::{ActiveTheme as _, Root};

use crate::config::Config;
use crate::editor::bind_keys;
use crate::tabs::{bind_tab_keys, WorkspaceShell};
use crate::theme::Palette;

fn main() {
    if let Err(err) = run() {
        eprintln!("crabmd: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse()?;
    if args.help {
        print_help();
        return Ok(());
    }
    if args.list_themes {
        for name in theme::list_theme_names() {
            println!("{name}");
        }
        return Ok(());
    }
    // Require a path first so a missing arg fails in the foreground.
    let raw_path = args
        .path
        .clone()
        .ok_or_else(|| anyhow::anyhow!("missing file path\n\n{HELP}"))?;
    if !args.wait {
        // Fast path: a live daemon opens a tab (~ms) and we exit. No file
        // I/O here so the shell returns immediately. Otherwise spawn the
        // daemon (detached child) which owns the socket from here on.
        let abs = absolutize(&raw_path);
        if daemon::try_forward(
            &abs.to_string_lossy(),
            args.line,
            args.col,
            args.behavior.as_str(),
        ) {
            return Ok(());
        }
        detach_and_reexec()?;
        return Ok(());
    }
    let path = args
        .path
        .clone()
        .ok_or_else(|| anyhow::anyhow!("missing file path\n\n{}", HELP))?;
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    let mut config = config::load();
    let palette = if args.theme_from_cli {
        theme::load_named(&args.theme)?
    } else {
        match theme::load_named(&config.theme) {
            Ok(p) => p,
            Err(_) => theme::load_named(theme::DEFAULT_THEME)?,
        }
    };
    if args.theme_from_cli {
        config.theme = palette.name.clone();
    }
    ensure_file(&path)?;
    let source =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let initial = match (args.line, args.col) {
        (Some(line), col) => Some((line, col.unwrap_or(1))),
        (None, _) => None,
    };
    launch(path, source, palette, config, initial);
    Ok(())
}

/// Spawn a detached `-w` child with the same args, then let the parent exit.
fn detach_and_reexec() -> Result<()> {
    let exe = std::env::current_exe().context("resolving crabmd executable")?;
    let mut child_args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    child_args.insert(0, "-w".into());
    let mut cmd = std::process::Command::new(exe);
    // Close stdio so panics/logs from the GUI child never leak back into the
    // launching terminal after the parent returns.
    cmd.args(&child_args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group so the shell job can end with the parent.
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().context("launching crabmd window")?;
    Ok(())
}

fn ensure_file(path: &std::path::Path) -> Result<()> {
    if path.exists() {
        anyhow::ensure!(path.is_file(), "{} is not a file", path.display());
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
    }
    std::fs::write(path, "").with_context(|| format!("creating {}", path.display()))?;
    eprintln!("created {}", path.display());
    Ok(())
}

fn launch(path: PathBuf, source: String, palette: Palette, config: Config, initial: Option<(usize, usize)>) {
    gpui_platform::application()
        .with_assets(crate::assets::Assets)
        // Stay alive with zero windows (daemon) so the next `crabmd file`
        // forwards over the socket instead of cold-booting. cmd-q quits.
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_component::init(cx);
            crate::assets::load_bundled_fonts(cx);
            bind_keys(cx);
            bind_tab_keys(cx);
            // Remote `http(s)` images (`img(SharedUri)`) download through
            // this client — without it GPUI uses a null client and every
            // remote photo silently never loads (same setup as GPUI's own
            // image example).
            if let Ok(client) = reqwest_client::ReqwestClient::user_agent("crabmd") {
                cx.set_http_client(std::sync::Arc::new(client));
            }
            crate::assets::apply_dock_icon();
            crate::editor::apply_palette(&palette, cx);

            cx.set_global(ShellRegistry { shells: Vec::new() });
            // Single-instance socket. `--wait` skips listening (blocking
            // one-shot); a lost race just opens without registering.
            let ipc_rx = daemon::start_listener();
            open_editor_window(path, source, palette, config, initial, cx);
            if let Some(rx) = ipc_rx {
                cx.spawn(async move |cx| {
                    loop {
                        while let Ok(req) = rx.try_recv() {
                            let _ = cx.update(|cx| handle_open(cx, req));
                        }
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(50))
                            .await;
                    }
                })
                .detach();
            }
            // cmd-q cleanup; a crash leaves a stale file, which the next
            // launch detects (connect fails) and replaces.
            let _quit_sub = cx.on_app_quit(|_| async { daemon::cleanup() });
            std::mem::forget(_quit_sub);
        });
}

/// Live shells + their windows for single-instance routing.
pub(crate) struct ShellRegistry {
    pub(crate) shells: Vec<(WeakEntity<WorkspaceShell>, AnyWindowHandle)>,
}

impl Global for ShellRegistry {}

/// Route one forwarded `crabmd <file:line:col>` into this process.
fn handle_open(cx: &mut App, req: daemon::OpenRequest) {
    let mut path = PathBuf::from(&req.path);
    path = std::fs::canonicalize(&path).unwrap_or(path);
    if ensure_file(&path).is_err() {
        return;
    }
    let Ok(source) = std::fs::read_to_string(&path) else {
        return;
    };
    let initial = match (req.line, req.col) {
        (Some(line), col) => Some((line, col.unwrap_or(1))),
        (None, _) => None,
    };
    let live: Vec<(WeakEntity<WorkspaceShell>, AnyWindowHandle)> =
        cx.update_global::<ShellRegistry, _>(|reg, _| {
            reg.shells.retain(|(w, _)| w.upgrade().is_some());
            reg.shells.clone()
        });
    let target: Option<(Entity<WorkspaceShell>, AnyWindowHandle)> = cx
        .active_window()
        .and_then(|a| {
            live.iter()
                .find(|(_, h)| h.window_id() == a.window_id())
        })
        .or_else(|| live.first())
        .and_then(|(w, h)| w.upgrade().map(|s| (s, *h)));
    match target {
        Some((shell, handle)) if req.behavior != "new" => {
            let _ = handle.update(cx, |_, window, cx| {
                shell.update(cx, |s, cx| {
                    s.open_tab(path, source, initial, window, cx);
                });
                window.activate_window();
            });
            cx.activate(true);
        }
        // `-n`, or no live window: new window, same process (no re-exec).
        _ => {
            let config = config::load();
            let palette = match theme::load_named(&config.theme) {
                Ok(p) => p,
                Err(_) => match theme::load_named(theme::DEFAULT_THEME) {
                    Ok(p) => p,
                    Err(_) => return,
                },
            };
            open_editor_window(path, source, palette, config, initial, cx);
        }
    }
}

fn window_options(cx: &mut App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(880.), px(1000.)), cx)),
        window_min_size: Some(size(px(520.), px(380.))),
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(point(px(9.0), px(9.0))),
        }),
        app_owns_titlebar_drag: true,
        app_id: Some("ai.blankeos.crabmd".into()),
        icon: crate::assets::window_icon(),
        ..Default::default()
    }
}

/// Open one OS window hosting a tab shell. Used by CLI launch, cmd-shift-n,
/// and (next) the single-instance daemon's `-n` path.
pub(crate) fn open_editor_window(
    path: PathBuf,
    source: String,
    palette: Palette,
    config: Config,
    initial: Option<(usize, usize)>,
    cx: &mut App,
) {
    let window_options = window_options(cx);
    cx.activate(true);
    cx.spawn(async move |cx| {
        cx.open_window(window_options, |window, cx| {
            window.activate_window();
            let shell = WorkspaceShell::view(path, source, palette, config, initial, window, cx);
            cx.new(|cx| Root::new(shell, window, cx).bg(cx.theme().background))
        })
        .expect("failed to open window");
    })
    .detach();
}

struct Args {
    help: bool,
    list_themes: bool,
    theme: String,
    theme_from_cli: bool,
    wait: bool,
    behavior: OpenBehavior,
    path: Option<PathBuf>,
    /// 1-based source line from `file.md:line[:col]` (zed-style).
    line: Option<usize>,
    /// 1-based source column from `file.md:line:col`.
    col: Option<usize>,
}

/// Zed-style open behavior. The default reuses the running window (fast
/// tab); `-n` forces a new window in the same process.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum OpenBehavior {
    New,
    #[default]
    Existing,
    Add,
    Reuse,
}

impl OpenBehavior {
    fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Existing => "existing",
            Self::Add => "add",
            Self::Reuse => "reuse",
        }
    }
}

impl Args {
    fn parse() -> Result<Self> {
        let mut help = false;
        let mut list_themes = false;
        let mut theme = theme::DEFAULT_THEME.to_string();
        let mut theme_from_cli = false;
        let mut wait = false;
        let mut behavior = OpenBehavior::New;
        let mut path = None;
        let mut line = None;
        let mut col = None;
        let mut iter = std::env::args().skip(1).peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-h" | "--help" => help = true,
                "--list-themes" => list_themes = true,
                "-w" | "--wait" => wait = true,
                "-n" | "--new" => behavior = OpenBehavior::New,
                "-e" | "--existing" => behavior = OpenBehavior::Existing,
                "-a" | "--add" => behavior = OpenBehavior::Add,
                "-r" | "--reuse" => behavior = OpenBehavior::Reuse,
                "--theme" | "-t" => {
                    theme = iter
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--theme requires a name"))?;
                    theme_from_cli = true;
                }
                flag if flag.starts_with("--theme=") => {
                    theme = flag.trim_start_matches("--theme=").to_string();
                    theme_from_cli = true;
                }
                arg if arg.starts_with('-') => {
                    anyhow::bail!("unknown flag `{arg}`\n\n{HELP}");
                }
                _ => {
                    if path.is_some() {
                        anyhow::bail!("unexpected extra argument `{arg}`");
                    }
                    let (file, l, c) = split_file_position(&arg);
                    path = Some(PathBuf::from(file));
                    line = l;
                    col = c;
                }
            }
        }
        Ok(Self {
            help,
            list_themes,
            theme,
            theme_from_cli,
            wait,
            behavior,
            path,
            line,
            col,
        })
    }
}

/// Make `path` absolute without touching the filesystem (the daemon does
/// `ensure_file` + read, so the fast CLI path stays pure).
fn absolutize(path: &std::path::Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

/// Split zed-style `file.md:line:col` (or `file.md:line`) into its parts.
/// Trailing `:digits` suffixes are treated as line/column; anything else is
/// kept verbatim as the file path (so `a:b.md:4` -> `a:b.md` line 4, while
/// `notes.md:abc` stays a plain path).
fn split_file_position(arg: &str) -> (String, Option<usize>, Option<usize>) {
    let mut rest = arg;
    let mut nums: Vec<usize> = Vec::new();
    for _ in 0..2 {
        let Some(ix) = rest.rfind(':') else {
            break;
        };
        let tail = &rest[ix + 1..];
        if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        let Ok(n) = tail.parse::<usize>() else {
            break;
        };
        nums.push(n);
        rest = &rest[..ix];
    }
    if rest.is_empty() {
        return (arg.to_string(), None, None);
    }
    match nums.len() {
        // Pushed col first, then line.
        2 => (rest.to_string(), Some(nums[1].max(1)), Some(nums[0].max(1))),
        1 => (rest.to_string(), Some(nums[0].max(1)), None),
        _ => (arg.to_string(), None, None),
    }
}

const HELP: &str = "\
crabmd — a fast native markdown writer

Usage:
  crabmd <file.md>
  crabmd <file.md>:<line>[:<col>]   (zed-style jump; hidden markup picks nearest)
  crabmd --theme <name> <file.md>
  crabmd -w <file.md>
  crabmd --list-themes

If <file.md> does not exist, an empty markdown file is created.

Single instance (zed-style): the first call owns the process (one dock
icon); later calls forward over a socket and exit in ~ms. Default opens a
tab in the running window; -n opens a new window in the same process.

Flags:
  -w, --wait     Block the terminal until the window closes (default: detach)
  -t, --theme    Theme name (see --list-themes)
  -n, --new      New window in the running process
  -e, --existing Open a tab in the existing window (default)
  -a, --add      Same as -e (tab in the focused window)
  -r, --reuse    Same as -e (reuses the window, no new process)
  -h, --help     Show this help

Themes (OpenCode JSON, default: from ~/.config/crabmd/config.toml or opencode):
  see --list-themes (all crabcode themes + grokday/groknight included)

Keys (Helix / Vim normal — tabbed buffers):
  cmd-alt-left/right  prev / next tab (ctrl-alt- works too)
  cmd-t         new tab (untitled)
  cmd-w         close tab (prompts when dirty)
  cmd-shift-n   new window
  cmd-shift-w   close window (prompts when any tab is dirty)
  cmd-q         quit the app (prompts per window, removes the socket)
  cmd/ctrl-s    save (explicit write; no autosave)
  cmd-shift-p   command palette (theme, editor, full width, source)
  cmd-shift-v   toggle markdown source view
  cmd-k t       theme picker (zed-style chord; cmd may stay held for t)
  cmd-,         settings
  cmd-f         find
  :w / :write   write from the command-line; :q closes the tab, :wq saves and closes it, :qa quits
  /             insert a GFM block (headings, lists, code, table, quote, hr, alerts)
  h/j/k/l       h/l stay on the line; j/k wrap-aware file lines
  w/b/e W/B/E   word / WORD; 0/^/$ line start / first non-blank / line end
  gg/G          first / last line of the document
  i/a  I/A      insert at caret / after, line start (first non-blank) / line end
  o/O           open line below / above inside the block
  v             Helix select / Vim visual; Vim V = visual line
  d             delete selection (Helix: also current char); Vim dd = line, D = to EOL
  c             change: visual deletes + insert; Vim cc, ciw/caw/ci-quote/ci-brace
  m             Helix match: miw/maw/mi-quote/ma-brace/... select object
  viw/vaw       visual inner/around word; v/c + i/a + quotes/parens/braces too
  > < =         indent / dedent / auto-indent: >> << ==, visual >, >j >G gg=G
  %             Helix select all (Vim: ggVG)
  x             Helix: select line (repeat extends). Vim: delete character
  u / U         undo / redo (Helix); Vim redo is ctrl-r
  escape        normal (collapse visual)
  drop/paste    image + video files beside the markdown file
";

fn print_help() {
    println!("{HELP}");
}

#[cfg(test)]
mod tests {
    use super::split_file_position;

    #[test]
    fn zed_style_positions() {
        assert_eq!(
            split_file_position("notes.md:10:3"),
            ("notes.md".to_string(), Some(10), Some(3))
        );
        assert_eq!(
            split_file_position("notes.md:10"),
            ("notes.md".to_string(), Some(10), None)
        );
        assert_eq!(
            split_file_position("notes.md"),
            ("notes.md".to_string(), None, None)
        );
        // Colons inside the path survive; only trailing numbers split off.
        assert_eq!(
            split_file_position("a:b.md:4"),
            ("a:b.md".to_string(), Some(4), None)
        );
        assert_eq!(
            split_file_position("notes.md:abc"),
            ("notes.md:abc".to_string(), None, None)
        );
    }
}
