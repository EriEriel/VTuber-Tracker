---
id: project: README
aliases: []
tags:
  - project
---

2026-07-16 Init 10:02

# goal
Building CLI client for Vtuber tracker backend that write in TS.

# stack
**Pure Rust**

## Crates
- **clap** for CLI
- **reqwest** for HTTP requests
- **serde_json** for JSON serialization and deserialization
- **tokio** for asynchronous programming

# tasks
- [x] Hitting API endpoint with HTTP requests
- [x] Deserializing JSON responses into Rust structs
- [x] Fetching and displaying all Vtubers in a user-friendly format

## Features
### Core CRUD operations

- [x] Create: Add new items to the database
- [ ] Read: Retrieve items from the database
  - [x] List all items, command: list / l
  - [ ] Search for items by specific criteria (name, org, platform, etc.), command: search / s
- [ ] Update: Modify existing items in the database
- [ ] Delete: Remove items from the database

# notes

# references

