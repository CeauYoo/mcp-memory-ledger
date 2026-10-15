# Local 30-scenario experience boundary
- This directory implements the user's explicit update: one hidden key input starts the full bounded run. Do not add SEND, alias approval, or per-call prompts.
- The assistant may run offline synthetic fixtures only. Never obtain/read a real credential, inspect the credential terminal, inspect environment/keychain/process memory, or launch authenticated calls.
- After the user requests result inspection, read the safe run summary first. Full local diagnostics require the user's authorization and must never be dumped into chat or uploaded.
- Use new synthetic ledger databases only. Do not alter user Codex configuration, project trust, existing memory, previous trials or their evidence. Runtime output belongs under the ignored target directory, never in tracked source.
- Retain every run and physical attempt. Do not automatically resume or retry whole runs. The active-live.lock is only a concurrent-run lock, not permission to discard prior attempt records.
- Full transcripts and diagnostics stay on this Mac. No Library/Git/upload of local trial records or credentials.
