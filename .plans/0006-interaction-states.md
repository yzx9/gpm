# 0006: Interaction state specification for all screens

**Priority:** P1
**Status:** TODO
**Phase:** Phase 2 (desktop Tauri app)

## What

Define interaction states (loading, empty, error, success, partial) for every UI feature across all 3 screens.

## Why

The design doc only describes happy paths. Users see loading spinners, empty lists, and network errors on mobile constantly. Without specifying these states, the implementer ships generic "Error occurred" messages that erode trust.

## Context

The eng review identified offline-first as a design gap. The test review identified 16 untested paths, many of which are error states. This spec closes both gaps by defining what the user SEES for every state.

Key states to specify:

- Setup: clone progress, auth failure, invalid identity
- Entry list: first load skeleton, empty repo ("No passwords yet"), search no matches, pull spinner, stale data indicator, network error + retry
- Entry detail: copy success toast with countdown, clipboard error, decrypt failure, empty notes, long notes

Design principles:

- Empty states need warmth + primary action + context (not just "No items")
- Error states need recovery path (not just "Error occurred")
- Loading states need progress indication (not just spinners)
- Toast messages show entry name but never password content

## Effort

~1h (human) / ~10min (CC)

## Depends on

Phase 2 (Tauri app wiring)
