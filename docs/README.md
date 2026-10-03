# True Tick Documentation

The True Tick documentation set combines written engineering guides with per-source Mermaid architecture diagrams, giving you two ways to explore the codebase. You can follow the prose guides for full explanations and rationale, or browse the visual layer for a quick structural overview of each module. Both layers describe the same project and are kept in sync so either path stays accurate.

## Written guides

Each guide covers a focused area of the project and assumes you are reading it alongside the code.

- [DEVELOPMENT.md](DEVELOPMENT.md): how the project is built, developed, and validated during normal work.
- [PORTABLE_PACKAGING.md](PORTABLE_PACKAGING.md): how the application is bundled, distributed, and run as a portable package.
- [UI_AND_INTERACTION.md](UI_AND_INTERACTION.md): the user interface, interaction model, and visible behavior of the application.
- [IPC_AND_CONFIGURATION.md](IPC_AND_CONFIGURATION.md): inter process communication and how runtime configuration is supplied and consumed.
- [ARCHITECTURE.md](../ARCHITECTURE.md): the parent guide that describes the overall system architecture and how the pieces fit together.

## Architecture diagrams

The visual layer holds one Mermaid diagram per source file, so each diagram maps to a single source module and mirrors the repository layout. This convention makes it easy to jump from a concern in code to its visual representation and back. The [diagrams/README.md](diagrams/README.md) index lists all 53 diagrams and points each one at its source file.

## Conventions

The whole documentation set is written to be accurate to the implemented code, never aspirational. Prose is the primary layer and carries the full explanation, while diagrams are the alternate visual view that reinforces the same structure. Each edit to code or diagrams is reflected in the other layer so that prose, visuals, and source stay consistent.