# brainstorm: refactor-logging-system

## Target
Refactor logging system: date-based file rotation, size limit (2MB auto-rotate), 7-day cleanup, release log markers.

## Requirements
1. Log file naming: .ikun_organizer_log_YYYYMMDD_N
2. Storage dir: ~/.ikun_organizer_log/
3. Size rotation: >2MB -> index+1
4. Cleanup: delete files >7 days old on startup
5. Release log macros: release_info!, release_warn!, release_error! with [RELEASE] prefix
6. Update CLAUDE.md and trellis spec

## Acceptance
- [ ] Files named correctly in ~/.ikun_organizer_log/
- [ ] Auto-rotate at 2MB
- [ ] Cleanup old logs on startup
- [ ] Release macros available
- [ ] Docs updated
- [ ] cargo check + clippy pass
