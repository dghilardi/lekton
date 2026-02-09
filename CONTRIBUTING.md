# Contributing to Lekton

Thank you for your interest in contributing to Lekton! We appreciate your help in making this project better.

## 🚀 Getting Started

1.  **Clone the repository:** `git clone https://github.com/dghilardi/lekton.git`
2.  **Install Rust:** Make sure you have the latest stable Rust installed.
3.  **Run tests:** `cargo test` to ensure everything is working correctly.

## 🛠️ Development Guidelines

### Workflow
We follow a **Trunk-Based Development** model. 
- Small, atomic commits.
- Feature branches for larger changes (keep them short-lived).
- All changes must pass CI (lints, tests).

### Coding Standards
-   **Linting:** Use `cargo clippy` and `cargo fmt`.
-   **Commit Messages:** Follow [Conventional Commits](https://www.conventionalcommits.org/).
-   **Documentation:** Document public APIs using doc comments (`///`).

## 🧪 Testing
- Write unit tests in the same file using the `#[cfg(test)]` module.
- Add integration tests in the `tests/` directory.

## 📝 Documentation
If you're updating documentation:
- Keep it concise and professional.
- Use GitHub Flavored Markdown.
- Update `CHANGELOG.md` to reflect your changes.

## ⚖️ Code of Conduct
Please read and follow our [Code of Conduct](CODE_OF_CONDUCT.md).
