# Changelog

All notable changes to `tpt-t-medical-mcp` are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate
uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- MCP supervisory interface over newline-delimited JSON-RPC 2.0.
- Read-only tools: `list_units`, `unit_status` and `audit_summary`.
- One journalled write: `request_assistance`, sent as the configured dispatcher.
- No reset, release, limit or veto tool. Inputs are validated strictly.
