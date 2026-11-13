# Encrypted Fields

Some text columns hold data that should not be readable in a database dump, such as API tokens, personal identifiers or notes. Mark those fields with `#[djangors(encrypted)]`. The ORM encrypts them with AES-256-GCM on every write and decrypts them when a row is loaded, so application code keeps working with plain `String` values.

Encryption is opt-in per field and works like Rails' `encrypts`.

## Setup

Mark a `String` or `Option<String>` field:

```rust,illustrative
use djangors_macros::Model;

#[derive(Model, Debug, Clone)]
#[djangors(app = "accounts", table_name = "accounts_integration")]
pub struct Integration {
    #[djangors(primary_key, auto)]
    pub id: i64,

    pub name: String,

    #[djangors(encrypted, max_length = 512)]
    pub api_token: String,

    #[djangors(encrypted)]
    pub webhook_secret: Option<String>,
}
```

Any other field type is a compile error. `encrypted` also cannot be combined with a relation.

Set the key in the environment before the first query that touches an encrypted field:

```text
DJANGORS_FIELD_ENCRYPTION_KEY=<base64 of exactly 32 bytes>
```

The key is read once, on first use. If it is missing or does not decode to 32 bytes, the operation returns `OrmError::InvalidQuery` with a message that names the variable but never prints the key.

## Generating a key

Either of these produces a suitable key:

```bash
openssl rand -base64 32
head -c 32 /dev/urandom | base64
```

Store the key in your secret manager or deployment environment. Do not commit it to the repository.

## Storage format

An encrypted value is stored in the ordinary text column as:

```text
enc:v1:<base64 of 12-byte random nonce || AES-256-GCM ciphertext>
```

Each save uses a fresh random nonce, so saving the same text twice produces different stored values. Because of this, the column must be wide enough for the ciphertext. Ciphertext is about 1.4 times the length of the plaintext plus 40 bytes, so a `max_length` of 100 needs a column of about 180 characters. The column type stays `TEXT` or `VARCHAR`; no special migration is needed to add the attribute to a new column.

Rows written before a field was marked `encrypted` contain plaintext. Those values are returned unchanged when loaded, so you can add the attribute first and re-save rows later to encrypt them.

## Key rotation

Configure keys with `DJANGORS_FIELD_ENCRYPTION_KEYS` as a comma-separated list of base64-encoded 32-byte keys, **newest first**:

```text
DJANGORS_FIELD_ENCRYPTION_KEYS=<newest key>,<previous key>[,<older key>...]
```

(The older two-variable form `DJANGORS_FIELD_ENCRYPTION_KEY` + `DJANGORS_FIELD_ENCRYPTION_OLD_KEYS` continues to work when `DJANGORS_FIELD_ENCRYPTION_KEYS` is unset.)

New writes always encrypt using the first key in `DJANGORS_FIELD_ENCRYPTION_KEYS` and tag each ciphertext with its key ID (`enc:v1:<key_id>:<base64>`). Reads look up the named key ID directly, while legacy untagged ciphertexts (`enc:v1:<base64>`) remain readable via key 0 and any older keys in the list.

### Batch re-encryption with `dj rotate-encryption-keys`

After prepending a new key to `DJANGORS_FIELD_ENCRYPTION_KEYS`, re-encrypt existing rows in transactional batches of 500 (default):

```bash
dj rotate-encryption-keys --model billing.PaymentMethod --batch 500
```

Each batch commits inside its own transaction and skips rows already tagged with the newest key ID, so an interrupted run can be resumed safely by running the command again. Once every row has been rotated, remove the retired keys from `DJANGORS_FIELD_ENCRYPTION_KEYS`. If any row references a missing or wrong key ID, decryption returns an error rather than exposing ciphertext.

## What you cannot do

Encryption uses random nonces, so the same plaintext never produces the same stored value. This has consequences:

- **Filtering is rejected.** `filter`, `exclude`, `filter_or_icontains`, `filter_datetime_range`, `after` and `search` on an encrypted field return `OrmError::InvalidQuery("cannot filter on encrypted field <name>")`.
- **Ordering is rejected.** `order_by` on an encrypted field returns the same error.
- **Lookups by value are impossible.** Use a separate, non-secret column, such as a keyed hash computed in your application, if you need to find a row by an encrypted value.
- **Uniqueness cannot be enforced on the plaintext.** A `unique` constraint on an encrypted column does not stop two rows from holding the same value.
- **Arithmetic updates are rejected.** `QuerySet::update` with an arithmetic `F` expression on an encrypted field returns an error.
- **Grouping and value aggregates are rejected.** `annotate` and `annotate_funcs` grouped by an encrypted field, `MIN`/`MAX`/`SUM`/`AVG` over one, and SQL functions such as `Lower`, `Upper`, `Concat`, `Length` and `Coalesce` on one return `OrmError::InvalidQuery`. `Count` still works, because it only checks for `NULL`.

`values()` and `values_list()` return encrypted fields decrypted, like a loaded model. Raw SQL is the exception: it returns the stored value (`enc:v1:...`), and `djangors_orm::encryption::decrypt_str` decrypts it.

Reads through `field_values`, the admin, JSON output and `write_json` use the decrypted value, so anything that serializes a model will expose the plaintext. Check your views before sending encrypted fields to clients.

## Backups and recovery

A database backup of encrypted columns is only useful together with the key that wrote it. Back up `DJANGORS_FIELD_ENCRYPTION_KEY` and every key in `DJANGORS_FIELD_ENCRYPTION_OLD_KEYS` alongside the database, in a store separate from the backups. Losing the key makes the encrypted values permanently unreadable. Restoring a backup without the key that was current when it was taken produces `OrmError::InvalidQuery` errors on load.

Test restores by loading an encrypted row with the restored environment.
