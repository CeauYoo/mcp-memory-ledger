## Problem and resulting behavior

Describe the concrete trigger, current problem and resulting behavior. Link the active-plan slice or issue. State whether the change is implemented, in progress or only documentation.

## Scope and compatibility

Describe changed read/write scope, workspace/personal grants, tool contracts, schema/config compatibility and recovery implications where relevant. Core workspace sessions must not silently expose global tools or automatic reflection. Describe any new permission explicitly.

## Verification

Record the exact source/binary identity, commands, OS/client and results, including failures and tests not run. Separate offline/mock, direct MCP protocol, real natural-language client, isolated package, fresh-machine and authorized live-model evidence. New core macOS/Linux evidence is separate from Windows legacy stdio regression. Do not inherit passing CI from another head.

## Boundaries and documentation

Link updated behavior/setup/testing documentation and the acceptance record. Note remaining limits and any deployment or release action that still requires user approval. A source PR is not a deployment or release.

- [ ] I reviewed the diff for credentials and private content. No raw database, private configuration, unrestricted log or user transcript is included.
- [ ] Verification claims match the evidence actually collected; mock and protocol checks are not presented as natural-language or live-model acceptance.
- [ ] Relevant documentation and compatibility/recovery notes are updated, or their non-applicability is explained above.
