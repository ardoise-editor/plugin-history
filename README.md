# Local History — an Ardoise Editor plugin

What files were, kept on this machine whatever Git knows, as IntelliJ IDEA
has it, for [Ardoise Editor](https://github.com/ardoise-editor/ardoise). A
file is kept when it is opened, saved, rewritten by a replace in files, about
to be deleted, and every minute while it is typed in.

**Local History: Show** (in the search for actions, and the menus of the
editor, of a file and of a folder) opens its panel: the revisions of the file
or folder, latest first, with the labels. Pick one to compare it with now,
put it back (or one of its changes), or save it as a patch. **Put label**
marks a moment: going back to a label puts back every file of the folder.

Its settings (Preferences › Plugins) say how many working days are kept.
Texts are stored once, by their hash, in the plugin's data folder.

## Build

```sh
cargo test
cargo build --release && mkdir -p bin && cp target/release/history bin/
```

## License

MIT
