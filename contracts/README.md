# IPC contract fixtures

These files are the committed boundary between Rust serialization and the React runtime schema. They contain aggregate, display-safe data only. Rust tests deserialize and round-trip them; Vitest validates them with strict Zod schemas and scans them for sensitive fields.

Snapshots expose the selected artifact format, six public processing stages, the stage at which a failure occurred, whether changed settings apply on the next run, typed automation settings, manual or automatic retirement mode, privacy-safe per-transmitter Trash outcomes, and whether the current log folder can be opened. They never expose absolute paths, device UUIDs, hashes, raw tool output, audio inspection metadata, ledger identifiers, or private proposal context.
