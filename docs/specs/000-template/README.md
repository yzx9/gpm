# Feature spec template

Copy this directory to start a new feature spec:

```
cp -r docs/specs/000-template docs/specs/<NNN>-<slug>
```

`NNN` is the next free 3-digit number; `<slug>` is a kebab-case feature name.

## Files

|File|Required?|Purpose|Lifecycle|
|-|-|-|-|
|`prd.md`|**Yes**|Product requirements: functional + non-functional + user characteristics|Living|
|`design.md`|No|Technical design & rationale ("why this approach")|Snapshot|
|`security.md`|No|Feature-level threat model (crypto / identity / lock / sync)|Living|
|`research.md`|No|Spike / investigation notes|Snapshot|
|`mockups/`, `diagrams/`|No|Wireframes / flow diagrams (add as needed)|Snapshot|

## Rules of thumb

- **Only `prd.md` is required.** Add companions only when the feature needs them; don't create empty placeholders.
- **PRD = requirements, not implementation.** How it's built → read the code/git. Keep "current state" to a few sentences.
- **Living vs snapshot:** `prd.md` / `security.md` stay equal to reality; `design.md` / `research.md` / `mockups/` are frozen once written.
- Named personas (Jordan / Casey) are defined in `docs/personas.md`.
