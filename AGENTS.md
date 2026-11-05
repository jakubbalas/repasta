# Repasta contributor instructions

- Read [PLAN.md](PLAN.md) before changing product scope, platform support, or
  data handling. Keep documentation and examples in Repasta's own vocabulary.
- Keep history and policy platform-neutral. Put native clipboard, shortcuts,
  and permissions behind adapters when those integrations are added. Report
  platform capabilities explicitly.
- Treat clipboard content and metadata as untrusted and potentially sensitive.
  Bound reads, queues, decoded sizes, and retained data. Keep clip contents out
  of logs and diagnostics. Use synthetic data in fixtures and benchmarks.
- Keep runtime behavior local. Review dependencies, native permissions,
  storage format, and data lifetime before expanding them.
- Keep capture and search responsive. Benchmark a representative workload when
  changing a hot path and record hardware, OS, build profile, and corpus.
- Run `cargo fmt --all -- --check`, `cargo clippy --locked --offline --all-targets -- -D warnings`,
  and `cargo test --locked --offline` before handing off changes. Include `--features desktop` for Clippy and tests
  when changing the desktop app. `make check` runs all of these. Exercise
  native integration changes on the platforms they affect and state which
  environments were exercised.
