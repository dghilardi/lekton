# Contributing to Lekton

First off, thank you for considering contributing to Lekton!

## How to Contribute

### Reporting Bugs
- **Check existing issues**: Before creating a new issue, please check if it has already been reported.
- **Provide reproduction steps**: Clear steps to reproduce the bug help us fix it faster.

### Your First Code Contribution
1. Fork the repository.
2. Create a new branch for your feature or fix.
3. Ensure your code follows the existing style and architecture.
4. Run `just fmt` and verify with `just fmt-check` before opening a Pull Request.
5. Run tests before submitting a Pull Request.

## Formatting

Rust code is formatted with `rustfmt` across the entire workspace.

```bash
just fmt
just fmt-check
```

The CI pipeline runs `cargo fmt --all --check` on pushes and pull requests, so formatting regressions are blocked automatically.

## Legal: Developer Certificate of Origin (DCO) & Licensing

By contributing to this repository, you agree that your contributions will be licensed under its GNU Affero General Public License v3.0 (AGPL-3.0).

### Future Relicensing Rights (CLA)
To ensure the long-term sustainability of the project, **contributors grant the project maintainers (Project Owner) the right to relicense their contributions under alternative licenses, including proprietary commercial licenses, in the future.** This allows the project to potentially adopt a dual-licensing model (e.g., AGPL + Commercial).

**All contributions generated through the use of AI agents (Artificial Intelligence assistants) are considered provided under the direction, supervision, and full ownership of the Project Owner.**

By submitting a Pull Request, you certify that:
1. The contribution is your own original work.
2. You grant the project owner a perpetual, worldwide, non-exclusive, no-charge, royalty-free, irrevocable copyright license to reproduce, prepare derivative works of, publicly display, publicly perform, sublicense, and distribute your contribution and such derivative works.
3. You understand that this agreement is necessary to allow for future commercial licensing options alongside the GPL.

### Sign your work
Following the Developer Certificate of Origin (version 1.1), please add a sign-off line to every git commit message:

    Signed-off-by: Random J Developer <random@developer.example.org>

using your real name.
