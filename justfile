# just is a command runner, Justfile is very similar to Makefile, but simpler.

default:
  @just --list

test:
  cd src-tauri && cargo test --all-features

lint:
  cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
  npx vue-tsc --noEmit

fmt:
  cd src-tauri && cargo fmt
  prettier --write src
