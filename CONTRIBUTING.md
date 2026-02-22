# Contributing to Scrappy

We welcome contributions from the community! Whether you are fixing a bug, adding a new feature, or improving documentation, your help is appreciated.

## How to Contribute

1.  **Fork the Repository**: Create your own copy of the project.
2.  **Create a Branch**: `git checkout -b feature/amazing-feature`.
3.  **Make Changes**: Ensure your code follows the existing style and pass all type checks (`npx tsc --noEmit`).
4.  **Commit Changes**: `git commit -m 'Add amazing feature'`.
5.  **Push to Branch**: `git push origin feature/amazing-feature`.
6.  **Open a Pull Request**: Describe your changes in detail.

## Development Workflow

- **Type Safety**: We use TypeScript strictly. Run `npx tsc --noEmit` locally before submitting.
- **Backend**: Rust code in `backend/` should follow standard idioms. Use `cargo check` to verify.
- **Aesthetics**: Scrappy is designed to be premium and professional. Ensure new UI components follow the "Glassmorphism" and "Rich Dark Mode" aesthetic guidelines.

## Bug Reports

Please use the GitHub Issue tracker to report bugs. Include:
- A clear description of the bug.
- Steps to reproduce.
- Your OS and hardware details (especially for AI/Metal related issues).

## License
By contributing, you agree that your contributions will be licensed under the project's **GNU General Public License v3.0**.
