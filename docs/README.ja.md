# MCP Memory Ledger

統合ローカル検証：Rust 1117 件成功、14 件はサンドボックスの AF_UNIX 初期化制限で失敗。Node 13 件、Python 87 件と厳格 Clippy は成功しました。ネイティブ受け入れ完了ではありません。[検証範囲](final-local-integration-validation-2026-10-20.md)を参照してください。

Rust + SQLite によるローカル優先の MCP `stdio` メモリサービスで、現在は技術 MVP です。証拠付きの記録、範囲別検索、訂正履歴、タスク継続、実行権限を持たない経験候補を扱います。[中国語 README](../README.md)を共通入口とし、[文書ナビ](document-map.md)、[クイックスタート](quickstart.md)、[ツール契約](tool-reference.md)を参照してください。

## 現在のローカル候補 · 2026-10-20

公開済みソースは引き続き `4ac6cfc` です。それ以降のローカル候補は**未公開**で、最終的な同一ソースのローカル検証は確認中、新しい正確なコミットの Ubuntu/macOS/Windows ネイティブ CI は未完了です。旧 CI や個別スライスの結果を今回の証拠に流用しません。[24 項目の計画](plans/2026-10-16-task-continuity-and-host-integration.md)はまだ全項目完了ではありません。既存 UI、三言語と権限境界は維持し、ユーザー環境へのインストールや配備はしていません。

- **Schema 11 のローカル所有者申告**：明示的な `task-owner-attest` が正確な要求、証拠、要求ごとの出所を保持し、通常の変更による保護対象要求の削除を防ぎます。移行には明示的なバックアップと予行検証が必要です。人間の本人認証、同一ユーザーの敵対的プロセスからの隔離、実行権限を意味しません。署名付きホスト取り込みは未実装です。[契約](local-owner-task-attestation.md)。
- **明示的な更新と出力記録**：controller refresh はタスク、範囲、状態、成果物を再確認します。アダプター schema 2 は正確な出力ダイジェストを保持し、旧状態からの移行は明示的です。出力記録はホスト受理やモデル採用の証明ではなく、バックグラウンド配信も主張しません。[契約](codex-task-context-adapter.md)。
- **任意の v2 意味索引と統合選択**：同一範囲の claims、events、永続 Episodes、現在有効な経験を明示的に構築・再開・リセットします。ホストが有効化する unified モードは必須要求を優先し、任意の知識に時間・バイト予算を共有します。種類ごとに有界な走査範囲と省略を報告し、全体 top-k を保証しません。読み取りによる自動再構築はなく、従来の字面検索と enhanced/hybrid モードも維持します。[索引](rebuildable-semantic-index.md) · [統合選択](unified-task-selection.md)。
- **型付き環境報告**：`reported_environment` は未認証の呼び出し側申告です。型と値の不一致は検索候補を除外できますが、不明な値は unknown のままです。ホスト方針、モデル設定、名前空間の権限は変更できません。[検索契約](enhanced-recall.md)。
- **T14 の最小結果 outbox 候補**：`codex-results` は明示的に関連付けた過去の use に限り、有界な出力メタデータを保存し、永続的な段階別送信と inspect/retry を行います。成功、因果的効果、権限を証明しません。ネイティブホストでの結果受理は未検証です。[契約](codex-result-feedback.md)。
- **配備パスの字句プレビュー**：対象プラットフォームのパス表記を確認するだけで、実ファイルシステムの安全性、同期対象かどうか、配備可能性は認証せず、インストールもしません。Windows hook プレビューと checked writes は引き続き未対応で、未対応環境にインストール可能な仮コマンドは出力しません。[プレビュー](deployment-path-preview.md) · [Windows の境界](windows-hook-preview-assessment-2026-10-20.md)。

公開済み基準の[明示的モデル草稿](experience-model-drafts.md)、[拡張検索](enhanced-recall.md)、任意の[ローカル Ollama と永続派生キャッシュ](local-embeddings.md)は維持します。standalone は 43 ツール、core は scoped 36 ツールを許可し、全体向け 7 入口を拒否します。モデル草稿は明示的許可を必要とし、pending 候補だけを生成します。ローカルの失敗は provider が未処理・未課金である証明ではありません。

## 証拠と制限

通常の recall は字面一致の契約を維持し、順位付けは事実や適用性を保証しません。JSON バイト数や合成履歴の空白区切り語数はモデルのトークン数ではありません。出所ラベル、候補の有効化、出力記録は実行権限を与えません。クラウドの AF_UNIX 制限、ネイティブ受理、fresh-machine 導入、実モデルの効果は別々に検証し、この候補の全テスト通過はまだ主張しません。

[現在の状態](project-status.md)、[検証方法](testing-guide-2026-03-24.md)、[復元時の記録](restored-feature-validation-2026-10-19.md)、[後続検証](follow-on-validation-2026-10-20.md)、[ホスト証拠の範囲](codex-native-host-verification.md)を参照してください。[唯一の active plan](plans/README.md)が実行順序を管理します。[全体 self-model の版管理](self-model-versions.md)は実験的で、正式公開には別途承認が必要です。実行ファイル名は `agent_llm_mm` です。
