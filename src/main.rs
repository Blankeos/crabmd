mod assets;
mod config;
mod coords;
mod display;
mod document;
mod editor;
mod images;
mod mode;
mod motion;
mod notion;
mod palette;
mod slash;
mod surface;
mod syntax;
mod theme;
mod tree;
mod undo;
mod wysiwyg;

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use gpui::{
    point, px, size, AppContext as _, Styled as _, TitlebarOptions, WindowBounds, WindowOptions,
};
use gpui_component::{ActiveTheme as _, Root};

use crate::config::Config;
use crate::editor::{bind_keys, Workspace};
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
    // Detach before any file I/O / theme load so the shell returns immediately.
    // Require a path first so a missing arg fails in the foreground.
    if args.path.is_none() {
        anyhow::bail!("missing file path\n\n{HELP}");
    }
    if !args.wait {
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
    launch(path, source, palette, config);
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

fn launch(path: PathBuf, source: String, palette: Palette, config: Config) {
    gpui_platform::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            bind_keys(cx);
            crate::assets::apply_dock_icon();
            crate::editor::apply_palette(&palette, cx);

            let window_options = WindowOptions {
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
            };

            cx.activate(true);
            cx.spawn(async move |cx| {
                cx.open_window(window_options, |window, cx| {
                    window.activate_window();
                    let view = Workspace::view(path, source, palette, config, window, cx);
                    cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
                })
                .expect("failed to open window");
            })
            .detach();
        });
}

struct Args {
    help: bool,
    list_themes: bool,
    theme: String,
    theme_from_cli: bool,
    wait: bool,
    path: Option<PathBuf>,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut help = false;
        let mut list_themes = false;
        let mut theme = theme::DEFAULT_THEME.to_string();
        let mut theme_from_cli = false;
        let mut wait = false;
        let mut path = None;
        let mut iter = std::env::args().skip(1).peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-h" | "--help" => help = true,
                "--list-themes" => list_themes = true,
                "-w" | "--wait" => wait = true,
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
                    path = Some(PathBuf::from(arg));
                }
            }
        }
        Ok(Self {
            help,
            list_themes,
            theme,
            theme_from_cli,
            wait,
            path,
        })
    }
}

const HELP: &str = "\
crabmd — a fast native markdown writer

Usage:
  crabmd <file.md>
  crabmd --theme <name> <file.md>
  crabmd -w <file.md>
  crabmd --list-themes

If <file.md> does not exist, an empty markdown file is created.

Flags:
  -w, --wait     Block the terminal until the window closes (default: detach)
  -t, --theme    Theme name (see --list-themes)
  -h, --help     Show this help

Themes (OpenCode JSON, default: from ~/.config/crabmd/config.toml or opencode):
  opencode, opencode-light, catppuccin, catppuccin-light,
  tokyonight, tokyonight-light, github, github-light

Keys (Helix / Vim normal — one buffer):
  cmd/ctrl-s    save (explicit write; no autosave)
  cmd-k         command palette (theme, editor, full width, source)
  cmd-,         settings
  cmd-f         find
  :w / :write   write from the command-line; :wq writes and quits
  /             insert a GFM block (headings, lists, code, table, quote, hr, alerts)
  h/j/k/l       h/l stay on the line; j/k wrap-aware file lines
  w/b/e W/B/E   word / WORD; 0/^/$ line start / first non-blank / line end
  gg/G          first / last line of the document
  i/a  I/A      insert at caret / after, line start (first non-blank) / line end
  o/O           open line below / above inside the block
  v             Helix select / Vim visual; Vim V = visual line
  d             delete selection (Helix: also current char); Vim dd = line, D = to EOL
  x             Helix: select line (repeat extends). Vim: delete character
  u / U         undo / redo (Helix); Vim redo is ctrl-r
  escape        normal (collapse visual)
  drop/paste    image files beside the markdown file
";

fn print_help() {
    println!("{HELP}");
}
