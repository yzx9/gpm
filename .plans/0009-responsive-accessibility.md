# 0009: Responsive layout and accessibility requirements

**Priority:** P2
**Status:** TODO
**Phase:** Phase 2 (desktop Tauri app)

## What

Define responsive behavior for 3 viewport sizes and accessibility requirements for all interactive elements.

## Why

Android-first means viewport handling is critical (software keyboard, notch, gesture bar, landscape). Accessibility is non-negotiable for a password manager — screen reader users need password managers too.

## Context

The app targets Android phones primarily, with desktop as bonus. The software keyboard takes 40-60% of screen on phones, which affects the setup page (3 inputs + button).

Responsive behavior:
- Phone (375px): Single column, full-width inputs, sticky bottom CTA on setup page
- Tablet (768px): Wider list, side padding, larger touch targets
- Desktop (1024px+): Max-width container (480px centered), same mobile layout

Accessibility:
- All interactive elements: minimum 48px touch target
- Screen reader: aria-live for copy confirmation, aria-label for show/hide toggle
- Keyboard: Tab order follows visual order, Enter submits, Escape navigates back
- Contrast: WCAG AA minimum (4.5:1 body, 3:1 large text)
- Security: autocomplete="off" on identity and PAT fields, input type="password"
- Focus: visible focus indicators for keyboard navigation

## Effort

~1h (human) / ~10min (CC)

## Depends on

Design system (0008)
