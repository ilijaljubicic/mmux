# mmux-controller-core

Runtime-neutral controller state for mmux.

This crate contains the node registry and command queue semantics shared by
controller runtimes. It intentionally avoids local runtime concerns such as
Axum, Tokio sockets, ractor, tmux, filesystem access, or local process
execution, so future controller runtimes such as Cloudflare Workers can reuse
the same behavior.

The native controller adapts this state behind its local runtime and actor
waiter map. A Worker runtime should provide its own persistence and request
routing, then call the same registry operations.
