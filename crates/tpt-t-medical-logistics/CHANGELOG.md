# Changelog

All notable changes to `tpt-t-medical-logistics` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Clean, dirty and isolation zone rules that require a complete UV-C cycle before
  re-entering a clean zone. Single-occupancy elevator with emergency priority.
- UV-aware route planning. A contaminated robot must pass a UV station before it
  re-enters clean space, and the search enforces that.
- Transport profiles that restrict what each load may enter.
- MQTT 3.1.1 QoS 0 framing, written in-house, with an elevator request, grant and
  release service. No client dependency is added.
- Transport destinations for admit and transfer events, and a clear on discharge,
  through the HL7 bridge. Unknown locations are refused.

### Known issues

- No door timing model and no traffic model between robots.
- No MQTT broker connection.
