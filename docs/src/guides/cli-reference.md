# CLI Reference (`dj`)

The `dj` binary (`djangors-cli`) is Djangors's equivalent of Django's `manage.py`. Install it from
crates.io:

```bash
cargo install djangors-cli
```

## Project & Development

| Command | Description | Example |
|---|---|---|
| `dj new <name>` | Scaffold a new Djangors project | `dj new mysite` |
| `dj new-app <name>` | Scaffold a new app module inside a project | `dj new-app blog` |
| `dj run [--port 8000]` | Watch files, rebuild on change, and reload browser | `dj run --port 8080` |
| `dj check [--deploy]` | Validate `djangors.toml`, system checks, and (with `--deploy`) production hardening | `dj check --deploy` |
| `dj diffsettings` | Print settings that differ from `DjangorsSettings::default()` | `dj diffsettings` |
| `dj showurls` | List all registered HTTP routes in `src/` | `dj showurls` |
| `dj test` | Run the project test suite (`cargo test`) | `dj test` |
| `dj shell` | Launch an interactive Rust REPL (`evcxr`) | `dj shell` |
| `dj dbshell` | Open `psql` / `sqlite3` against `DATABASE_URL` | `dj dbshell` |
| `dj seed [--force]` | Run `src/bin/seed.rs` to populate dev data | `dj seed` |

## Database, Migrations & Fixtures

| Command | Description | Example |
|---|---|---|
| `dj makemigrations [--check]` | Introspect models and generate numbered SQL migrations | `dj makemigrations` |
| `dj migrate [--rollback [N]] [--plan] [--fake]` | Apply, preview, fake, or roll back migrations | `dj migrate --plan` |
| `dj sqlmigrate <app> <migration>` | Print SQL statements for a migration without executing | `dj sqlmigrate blog 0001` |
| `dj showmigrations` | List all migrations and their applied status | `dj showmigrations` |
| `dj squashmigrations <start> <end> [--name ...] [--dir migrations]` | Combine a contiguous range of SQL migrations into one | `dj squashmigrations 0001 0005 --name 0001_squashed_0005` |
| `dj inspectdb [--database-url ...] [--output ...] [--app app]` | Introspect an existing DB and emit `#[derive(Model)]` structs | `dj inspectdb -o src/models.rs` |
| `dj dumpdata [tables...] [-o file.json] [--database-url ...]` | Export table rows as Django-style JSON fixtures | `dj dumpdata auth_user -o users.json` |
| `dj loaddata <files...> [--database-url ...]` | Insert JSON fixture records into the database | `dj loaddata fixtures/initial.json` |
| `dj flush [--no-input] [--database-url ...]` | Delete all rows from non-migration tables | `dj flush --no-input` |
| `dj createcachetable [table] [--database-url ...]` | Create the table required by `DatabaseCache` | `dj createcachetable djangors_cache_entries` |

## Auth, Sessions, Security & Email

| Command | Description | Example |
|---|---|---|
| `dj createsuperuser --username <u1> [--email ...] [--skip-validation]` | Create an admin superuser in `auth_user` | `dj createsuperuser --username admin` |
| `dj changepassword <username> [--password ...] [--database-url ...]` | Hash (Argon2id) and update a user's password in `auth_user` | `dj changepassword alice --password '...'` |
| `dj createpermissions` | Sync `view`/`add`/`change`/`delete` permissions for all models | `dj createpermissions` |
| `dj clearsessions [--database-url ...]` | Delete expired server-side sessions from `djangors_session` | `dj clearsessions` |
| `dj rotate-encryption-keys --model <app.Model> [--batch 500]` | Re-encrypt `#[djangors(encrypted)]` columns with the primary key | `dj rotate-encryption-keys --model billing.Card` |
| `dj sendtestemail <recipients...> [--subject ...]` | Verify email configuration and send a test message | `dj sendtestemail ops@example.com` |

## Code Generation, Static Files, i18n & Workers

| Command | Description | Example |
|---|---|---|
| `dj generate resource <Name> [field:type...]` | Scaffold model, migration, REST ViewSet, admin, and tests | `dj generate resource Post title:string body:text` |
| `dj openapi [--typescript] [-o file] [--routes routes.json]` | Generate OpenAPI 3.1 JSON or a typed TypeScript fetch client | `dj openapi --typescript -o src/api.ts` |
| `dj collectstatic [--source static] [--output staticfiles]` | Hash and collect static assets + `manifest.json` | `dj collectstatic` |
| `dj makemessages [-l en] [--output-dir locales]` | Scan `templates/` and `src/` for `\|trans` & `t!()` keys into `.ftl` | `dj makemessages -l fr` |
| `dj compilemessages [--dir locales]` | Validate syntax and balanced braces in all `.ftl` catalogs | `dj compilemessages` |
| `dj runworker [--poll-interval-secs 1]` | Run the background task worker loop | `dj runworker` |

## Custom Management Commands

Any unrecognized subcommand `dj <my-command> [args...]` is automatically routed to your project's
binary with `DJANGORS_RUN_COMMAND=<my-command>`, matching functions annotated with
`#[management_command]`.
