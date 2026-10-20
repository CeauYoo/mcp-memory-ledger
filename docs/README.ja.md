# MCP Memory Ledger

Rust + SQLite によるローカル優先の MCP `stdio` メモリサービスで、現在は技術 MVP です。証拠付きの記録、範囲別検索、訂正履歴、タスク継続、実行権限を持たない経験候補を扱います。[中国語 README](../README.md)を共通入口とし、[文書ナビ](document-map.md)、[クイックスタート](quickstart.md)、[ツール契約](tool-reference.md)を参照してください。

## 現在の復元スライス

Schema 10、standalone 43 ツール（scoped 36、全体向け 7 入口は拒否）です。[明示的なモデル草稿](experience-model-drafts.md)、[中国語分割・同一範囲の別名・RRF 検索](enhanced-recall.md)、適用性の申告、任意の[ローカル Ollama](local-embeddings.md)、ホスト設定による literal/enhanced/hybrid のタスク検索を含みます。永続的な派生キャッシュは実装中で、検証済みではありません。既存 UI に長い説明や新たな書き込み権限は追加しません。

復元した第三スライスは再検証中です。基準 `8f87a813` は Ubuntu / Windows CI と Ubuntu の実 Codex startup/resume 合成送達 gate を通過しましたが、その版に限る証拠です。当時の macOS `/var` fixture 問題は現ソースで修正し、新しいネイティブ CI を待っています。[現在の状態](project-status.md)と[ホスト検証範囲](codex-native-host-verification.md)を確認してください。過去の 990 件などの結果を今回に流用しません。

新しい 30 組の合成シナリオは計 756 ターンで、長い履歴 2 本は空白区切りで 20,478 / 19,738 語です。トークン数でも実モデルの効果測定でもありません。ユーザー環境へのインストールや配備はしていません。

通常の recall は字面一致の契約を維持します。任意の意味ランキングは有界で、事実や適用性を保証しません。モデル草稿は明示的な許可と永続予約を必要とし、pending 候補だけを生成します。ローカルのエラーは、遠隔 provider が未処理・未課金である証明ではありません。有効化も実行権限を与えません。

[唯一の active plan](plans/README.md)に沿って進めます。[全体 self-model の版管理](self-model-versions.md)は実験的機能です。正式公開、fresh-machine 導入、実モデルの効果は別途検証します。実行ファイル名は `agent_llm_mm` です。
