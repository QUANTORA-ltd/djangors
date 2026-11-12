# djangors-contrib-orgs

Organizations, teams, roles, and memberships for Djangors.

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-orgs = "0.13"
```

Adds `Organization`, `Membership`, `Team`, and `TeamMembership` models, a role-to-permission map (`OrgPermissions`), the `CurrentOrg` extractor, and invitation helpers built on `djangors-auth`. An organization always keeps at least one owner.

- Documentation: https://docs.rs/djangors-contrib-orgs
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
