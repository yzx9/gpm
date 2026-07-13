# Storage backend state model — who owns the root

**Priority:** P1
**Status:** Draft
**Phase:** Next

## What

Today every storage backend is stateless: each content operation receives the repository root as a per-call argument, and the backend holds nothing between calls. This RFC proposes flipping the model so a backend owns its root as construction-time state, and operations are invoked against the backend instance rather than against a fresh root handed over each time. The decision is recorded separately from the individual backend RFCs because it is the foundational choice the whole pluggable-backend family builds on — and the current stateless model was never a deliberate boundary, it was the shape the one backend happened to take.

## Why

The stateless model rests on two premises that held when there was exactly one backend and its root was always a real filesystem path: the root is cheap to pass per call, and the root is a value the caller already has on hand. Both break for the backends on the roadmap.

The cloud-folder backend's root is a document-tree URI, not a path. Turning it into something each operation can use means acquiring a persistable permission and holding the resulting handle for the backend's lifetime — that handle cannot be re-acquired cheaply on every operation, and it is not representable as the per-call argument type the current model threads through every method. A backend that must hold such a handle is, by construction, stateful.

The filename-encrypting backend is a decorator that wraps another backend and layers name encryption over it. A decorator must hold the backend it decorates; that is state. The provenance-verification backend similarly holds the signing policy it enforces. Neither fits a model where a backend is a bag of stateless operations keyed by an externally-supplied root.

Beyond fit, the stateless model forces the store to re-read the persisted config on every single content operation just to recover the root it already knew at resolve time. That is wasted work today, and it becomes nonsense once the root is a held handle rather than a string.

## Context

The current model is not a designed boundary — it is an artifact of the first implementation phase. With only the git backend, whose root is a plain directory and whose operations are thin wrappers over a libgit2 call, "pass the path each time" was the path of least resistance and happened to align with gopass's storage interface, which also keys operations on an externally-supplied repository path. gopass gets away with that because every gopass backend's root is a real filesystem location; gpm is about to grow one whose root is not.

The registration mechanism — the work that lands the seam a backend rides in on — was deliberately built to host this transition without dictating it. The registry resolves a backend from a persisted type plus an opaque root token, and for the git built-in that root token is accepted and then ignored, precisely because the trait has not yet decided to let the backend consume it. This RFC is the decision that the backend should.

gopass's own architecture supports either reading. Its backends are nominally keyed by a path, but each backend is also a long-lived object that holds its configuration — a remote URL, a recipient list, an SCM choice. The "path as the operation key" surface and "backend as a held, configured object" interior coexist there; the question here is which side owns the root token, and gopass's interior is closer to what the non-filesystem backends need.

The threat model is unchanged in direction. The root token already lives in sealed, at-rest config and is only ever handed to a backend the app registered as trusted. Moving its consumption from per-call into construction widens a backend's reachable state from "one operation's worth" to "the backend's lifetime" — which is exactly what a held handle requires — and changes no trust boundary, since the backend was already trusted for the whole repository for the whole session.

## Alternatives considered

**Keep the stateless trait; give the cloud-folder backend its handle out of band.** The backend would carry the handle in a side channel keyed by the root string, looking it up per operation. This preserves the trait surface but moves the held state into a global map the backend crate cannot name without the app layer's types — re-introducing exactly the cross-layer coupling the registration seam exists to dissolve — and leaves the decorator and provenance backends still unaddressed. Rejected: it solves one backend by smuggling state instead of admitting the model needs state.

**Make the root a per-backend type the trait passes opaquely.** Each backend declares its own root type and the store threads the right one. This keeps the store as the owner of the root but forces the store to know a backend-specific type, which the registration seam was built to avoid — the store resolves a backend without learning what its root is — and it still re-reads config on every operation. Rejected for the same coupling reason.

**A callback / inversion backend where the store asks the app layer per file operation.** This is the inversion the filesystem-backend RFC already rejects at a larger scale: it duplicates the store's orchestration into the app layer. Rejected.

**Do nothing; let each backend RFC reshape the trait locally.** The filesystem, decorator, and provenance RFCs each reach the same stateful conclusion on their own. Letting them diverge risks three incompatible reshapes and three rounds of churn across the same content-operation call sites. This RFC exists to make the decision once, up front, so the family shares one trait.

## Effort

~1–2 days (human) / ~moderate CC. Dominated by reworking the content-operation call sites that currently thread the root per call, and by teaching the resolve path to hand the root to the backend once at construction. No new backend ships here — this is the trait move the subsequent backends stand on.

## Depends on / Supersedes

Depends on the storage-backend registration mechanism — the seam that already resolves a backend from a persisted type and root token and that, today, ignores the token for the git built-in. This RFC is the decision to consume it.

The pluggable filesystem backend, the filename-encrypting decorator, and backend-owned provenance verification each then build on the stateful trait this establishes.
