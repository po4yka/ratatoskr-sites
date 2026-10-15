# Privacy fixture vault

`vault/` holds the files of one vault connection, and `index.json` what the notes internal index
route gives for each file, without the revision (`fixtures/selection-vault/README.md`). The
publication of `crates/release/tests/privacy.rs` reads the whole vault in the mode `all` and
excludes `private/`; `Diary zq7diaryname.md` has `publish: false`.

Every string that contains `zq7` comes from a file that the publication does not select: a file
name, a frontmatter `title`, a frontmatter field, a heading, a block id, a tag, a body and the bytes
of `img/plan.png`, which only an unselected note embeds. No byte of a release may contain `zq7`.
The strings with `yk4` are in `b.md` and `c.md`, which the rebuild tests remove from the
publication. A new filter rule adds its canary location here (workspace design, Risks).
