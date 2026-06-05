# 0008: Minimal design system for agepass MVP

**Priority:** P1
**Status:** TODO
**Phase:** Phase 2 (desktop Tauri app) — create before building UI

## What

Define a minimal design system: color variables, typography scale, spacing system, and touch target requirements for the 3-page app.

## Why

No DESIGN.md exists. Without a design system, every component gets ad-hoc styling. Colors drift, spacing is inconsistent, fonts vary. For a trust-focused product, visual inconsistency undermines credibility.

## Context

The app is 3 pages (Setup, Entry List, Entry Detail). The design system is deliberately minimal — dark theme for trust, single accent color for focus, monospace for crypto content.

Color system (CSS variables):
- --bg-primary: #0D1117 (deep charcoal)
- --bg-surface: #161B22 (card/surface background)
- --text-primary: #E6EDF3 (high contrast)
- --text-secondary: #8B949E (metadata)
- --accent: #58A6FF (single blue for CTAs)
- --danger: #F85149 (errors)
- --success: #3FB950 (success states)

Typography:
- Display: Inter 600 20px (app name)
- Heading: Inter 600 16px (entry names)
- Body: Inter 400 14px (notes, metadata)
- Mono: JetBrains Mono 14px (identity input, paths)

Spacing: 8px base unit, 16px screen padding, 12px list item padding
Touch targets: minimum 48px height, full-width primary buttons

## Effort

~30min (human) / ~5min (CC)

## Depends on

None — create as a foundation before Phase 2
