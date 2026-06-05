# 0004: Add git commit signature verification / repo authenticity

**Priority:** P3
**Status:** TODO
**Phase:** Future

## What

Verify commit signatures or pin the remote host to prevent a compromised server from feeding encrypted garbage.

## Why

Outside voice (Codex) identified that there is no provenance check beyond "git pull succeeded." A malicious or compromised remote can feed perfectly valid encrypted entries that decrypt to wrong data. The user has no way to detect this.

## Context

This is a genuine trust gap for a product whose value proposition is trust. However, solving it properly requires:

- Commit signature verification (GPG or SSH signed commits)
- Key distribution story (how does the user know which signing key to trust?)
- Trust anchor management (first-use trust on commit? pinned key?)

The gopass ecosystem doesn't have a standard for commit signing. This needs design work before implementation.

## Effort

~2-3 days (human) / ~1 hour (CC) for design + implementation

## Depends on

None — can be added to any phase. Requires design doc first.
