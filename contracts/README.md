# IPC contract fixtures

These files are the committed boundary between Rust serialization and the React runtime schema. They contain aggregate, display-safe data only. Rust tests deserialize and round-trip them; Vitest validates them with strict Zod schemas and scans them for sensitive fields.
