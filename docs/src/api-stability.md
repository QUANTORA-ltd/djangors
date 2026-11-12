# API Stability & Deprecation Policy

This page states, for real consumers of Djangors (not just internal contributors), what
compatibility guarantee the project makes, how it changes public APIs, and how it ships releases.
It exists because none of this was published anywhere discoverable before. The intent lived only
in internal planning notes.

## Versioning

Djangors follows [Semantic Versioning](https://semver.org/) starting at `1.0.0`. All workspace
crates (`djangors`, `djangors-core`, `djangors-orm`, `djangors-rest`, and the rest) share a single
version number via the workspace's `[workspace.package]` version, and are released together. You
will never see `djangors-orm` at `1.3.0` while `djangors-core` is at `1.1.0`. Before `1.0`, expect
breaking changes between minor (`0.x`) releases without a deprecation cycle; the guarantees below
apply from `1.0` onward.

**What "the public API" means**: across Freeze Review Pass 1 and Pass 2 (`docs/design/10.14-api-freeze-pass-2.md`), this policy covers the core framework and primary battery crates: `djangors-core`, `djangors-orm`, `djangors-rest`, `djangors-db`, `djangors-auth`, `djangors-sessions`, `djangors-admin`, `djangors-forms`, `djangors-views`, `djangors-template`, `djangors-migrations`, `djangors-cache`, `djangors-mail`, `djangors-tasks`, `djangors-i18n`, `djangors-staticfiles`, `djangors-pdf`, and `djangors-test`. The `djangors` facade crate (`djangors::core`, `djangors::orm`, `djangors::rest`, etc.) re-exports these behind granular feature flags and is the intended single entry point for applications. Public configuration structs that are expected to grow new fields over time (such as `DjangorsSettings`, `djangors_mail::Message`, and `djangors_rest::ViewSetConfig`) are marked `#[non_exhaustive]` with builder constructors so adding fields in minor releases never breaks downstream code.

## Deprecation mechanics

- A public item slated for removal is marked `#[deprecated(since = "X.Y.Z", note = "...")]` with a
  note explaining the replacement or reason.
- It remains functional, with the deprecation warning, for **at least one full minor release
  cycle** before it can be removed in a subsequent minor or major release. It is never removed in
  the same release it was deprecated in.
- Every release with deprecations or removals lists them explicitly under their own "Deprecated"
  and "Removed" headings in `CHANGELOG.md` (see below), not buried in a generic changelog
  paragraph.

## `CHANGELOG.md`

A root [`CHANGELOG.md`](https://github.com/Chidi09/djangors/blob/main/CHANGELOG.md) is maintained
in the [Keep a Changelog](https://keepachangelog.com/) format (`Added`/`Changed`/`Deprecated`/`Removed`/`Fixed`/`Security` sections per tagged release, plus `[Unreleased]`), alongside a historical summary of earlier pre-release development phases.

## Release cadence

A tagged release ships roughly every 4-6 weeks, each with a changelog entry. This is not a new
commitment invented for this doc. It restates a cadence the project has intended internally since
early planning (`PLAN.md`'s own "Project operations" section); this page makes it visible to
actual consumers rather than leaving it as an internal note.

## Changing the public API

Any change to the public API surface at or above "medium" impact (renaming or removing a public
item, changing a trait's required methods, changing a function's signature in a way existing
callers must adapt to) goes through a short RFC first: a design doc under `docs/design/` describing
the change and its motivation, open for comment before landing. This mirrors the process already
used internally for every architecturally significant change in this project's own history (every
slice in `docs/design/` is exactly this kind of doc). Formalizing it here means outside
contributors can propose and review API changes the same way, not just watch them happen.

## Pre-1.0 Scope Note

While Freeze Review Pass 1 and Pass 2 have audited the core and primary battery crates (including `#[non_exhaustive]` hardening and unified `&Database` model CRUD signatures), Djangors is still pre-`1.0`. Before `1.0.0` is tagged, breaking changes may still land between `0.x` minor releases when documented in `CHANGELOG.md` and `docs/design/`.
