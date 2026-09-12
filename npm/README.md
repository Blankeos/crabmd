# crabmd (npm retired)

> The npm package is retired. New releases are NOT published to npm.
> Existing versions remain installable but will not receive updates.
>
> - macOS (supported): `brew install --cask blankeos/tap/crabmd`
> - Linux: `curl --proto "=https" --tlsv1.2 -LsSf https://github.com/Blankeos/crabmd/releases/latest/download/crabmd-installer.sh | sh`
>
> See the [GitHub README](https://github.com/Blankeos/crabmd) for the
> supported installs and for migrating from npm (settings are kept).

A fast native markdown **writer** for one person. One window, one file, write
back to disk. You type in the rendered document, not in a source buffer.

## Migrate from npm to the cask (macOS)

Settings in `~/.config/crabmd` are retained. Run the desktop uninstall
BEFORE removing the npm binary:

```sh
crabmd --uninstall-desktop
npm uninstall -g crabmd
brew install --cask blankeos/tap/crabmd
```

```sh
crabmd notes.md
```

See the [GitHub README](https://github.com/Blankeos/crabmd) for editors, keymaps, and GFM details.
