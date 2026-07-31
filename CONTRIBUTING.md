# Contributing to daisy-embassy

Thank you for your interest in contributing to this project!

## Code of Conduct

We strive to create an open and welcoming community for everyone. Let’s collaborate with respect and good manners to build an awesome space for audio development! Above all, we *LOVE* audio development-so let’s make some killer noise together!

This is a small, collaborative project maintained by a few developers. We aim to keep written rules minimal and prefer to make decisions through open conversation when needed. Your help is always welcome.

## Basic Guidelines

- **If you're unsure about something, feel free to ask.**  
  Opening an issue or a draft PR just to check your direction is totally fine.

- **It helps a lot if you drop a quick note before starting to write code.**  
  Especially for large changes or new APIs, it's great to align on direction beforehand.

- **Use `cargo fmt` for code formatting.**  
  This helps keep diffs clean and consistent.

- **CI will run the following checks:**

```bash
  cargo fmt -- --check
  cargo clippy --features seed -- --deny=warnings
  cargo clippy --features seed_1_1 -- --deny=warnings
  cargo clippy --features seed_1_2 -- --deny=warnings
  cargo clippy --features seed3 -- --deny=warnings
  cargo clippy --features patch_sm -- --deny=warnings
```

Please make sure your code passes these checks. If you're having trouble, feel free to mention it.

## Testing and Hardware Verification

**Before opening a PR, please try running the relevant examples under `examples/` on real hardware.**

In your PR comment, include:

- Which version of Daisy Seed you tested with (e.g., rev 1.1 or rev 1.2)
- Which examples you ran
- Whether they succeeded or failed

If you don’t have access to hardware, just mention it—we’ll try to help verify on our side if possible.

## License

This project is licensed under the MIT License.
By submitting code via pull request, you agree to license your contribution under the MIT License.

---

If you have any suggestions or questions, don’t hesitate to open an issue or PR. We're glad to have your support!
