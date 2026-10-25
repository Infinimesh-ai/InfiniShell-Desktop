# How to perform migrations

## How do migrations work?
A sqlite database is one single file on the user's computer, and instead of being a separate process, it's a set of C functions
that are bundled in our app. When the warp app starts, we upgrade the schema to the latest version in a transaction.
Since we don't control the machine, we need to be super careful about the migrations that we ship.

TODO: C.I. and remediation of failed migrations

## Step 1: One-time setup
Complete `./script/bootstrap` as described in the development guide.
`script/install_cargo_build_deps` installs `diesel_cli` through `cargo binstall`;
it no longer installs the old private fork and skips this developer tool in GitHub Actions.
Check `diesel --version` and SQLite support before running migrations.

Additionally, the `sqlite3` binary is useful in development. The one on your Macbook is ok, but
likely will be missing some basic language features. You can grab the latest binary from the
official website https://www.sqlite.org/download.html. Note that we use SQL language features that
are not available on old versions of sqlite. Note that you'll have to approve it to be run in your mac system preferences.

## Step 2: Write the migration
Run from the repository root with an explicit migrations directory:
```sh
diesel migration generate --migration-dir crates/persistence/migrations <descriptive_name>
```
This creates a directory containing `up.sql` and `down.sql` under `crates/persistence/migrations/`.

## Step 3: Run the migration + generate the schema
Read the local test rules in the root `AGENTS.md`. Choose an isolated development database or a backed-up database copy and assign its absolute path to `INFINISHELL_MIGRATION_DB`. Do not experiment on the database used by your daily application; its location varies by platform and channel.

Run from the repository root:
```sh
diesel migration run --migration-dir crates/persistence/migrations --database-url="$INFINISHELL_MIGRATION_DB"
```
The root `diesel.toml` writes the generated schema to `crates/persistence/src/schema.rs` and applies `crates/persistence/schema.patch`. The app re-exports it through `app/src/persistence/mod.rs`; do not maintain manual edits in the generated file.

You can also print the schema from a database that already has the migration with:
```
diesel print-schema --database-url="$INFINISHELL_MIGRATION_DB"
```

## Reverting/redo-ing migrations
As you are writing features and changing branches, you'll want to undo migrations to fix your database and make it compatible with older code. Redo-ing can also be helpful as you are iterating on your schema.
```
diesel migration revert --migration-dir crates/persistence/migrations --database-url="$INFINISHELL_MIGRATION_DB"
diesel migration redo --migration-dir crates/persistence/migrations --database-url="$INFINISHELL_MIGRATION_DB"
```

# Schema style
- We use `id` for integer primary keys. If there's something more special about the primary key, consider a more descriptive name.
- Plural table names, singular for structs in the rust model code.
- If there's a table `foos` and a table `bars`, and `bars` has a column with a foreign key that references `foos.id`, name it `foo_id`.

# The `schema.patch` file
The `crates/persistence/schema.patch` is manually updated by us. This file lets us make manual
changes on top of the auto-generated `schema.rs` file produced by `diesel_cli`.

To create the `schema.patch` file, we:
1. Run the diesel migrations
1. Manually edit `schema.rs`
1. Run `git diff -U6 > crates/persistence/schema.patch`.

You can read more about this patch file in the official [Diesel documentation](https://diesel.rs/guides/configuring-diesel-cli.html#the-patch_file-field).
