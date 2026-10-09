# MCP Memory Ledger プロジェクト概要

## 要約

MCP Memory Ledger は、長期記憶・自己スナップショット・反省更新の最小ループを検証するための Rust 製ローカル MCP `stdio` memory demo です。現在の実装は SQLite を永続化基盤としており、完成した製品というより、技術デモ、統合プロトタイプ、研究向け MVP として位置付けるのが適切です。

互換性メモ：現在の Rust crate、binary、script、設定例、および一部の履歴ドキュメントでは、技術識別子として `agent_llm_mm` / `agent-llm-mm` がまだ使われています。公開プロジェクト名は MCP Memory Ledger に統一します。

## 現在のスコープ

現在の 31 ツールと能力は[実装状態](project-status.md)と[ツール索引](tool-reference.md)に集約しています。schema6、フィードバック訂正、字面検索・context、経験候補の版管理は実装済みです。実ユーザークライアント、fresh-machine、実モデル効果、公開承認の gate は未完了です。

## 適した用途

- ローカル AI クライアント統合の検証
- self-agent memory の技術デモ
- Rust + MCP + SQLite の最小構成リファレンス

## ドキュメント運用ルール

- 各タスクの完了後、そのタスクが挙動、能力境界、接続手順、設定、検証コマンド、または協業ルールに影響する場合は、対応するドキュメントを必ず同時に更新します。
- ドキュメント更新を最後にまとめて回すのではなく、可能な限りコード変更と同じタスク内で一緒に収束させます。
- [正式化改善計画](formalization-improvement-plan-2026-08-25.md)は gap と acceptance の対応表であり、第二の実行キューではありません。現在は `CeauYoo/dev_work_dots` から上流 `dev-work` への draft PR を利用します。merge/release には別途明示的な承認が必要です。

## 現在の検証状態

現在の検証は[実装状態](project-status.md)、過去の記録は[日付付き履歴](project-status-history-2026-10-09.md)を参照してください。

## 謝辞

このリポジトリは、OpenAI Codex を協調的な開発ツールとして活用しながら、実装・議論・文書整理を進めてきました。このようなワークフローを可能にするツール群と研究エコシステムを提供している OpenAI に感謝します。
