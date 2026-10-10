# MCP Memory Ledger

Rust + SQLite によるローカル優先の MCP `stdio` メモリサービスです。証拠付きの記録、namespace ごとの検索、履歴を残す Claim の訂正、バージョン管理された経験候補を扱います。

最新の共通入口は[中国語 README](../README.md)です。[文書ナビゲーション](document-map.md)、[クイックスタート](quickstart.md)、[MCP 接続設定](local-mcp-integration-2026-03-26.md)、[実行可能なワークフロー](runnable-memory-workflow.md)、[ツール一覧](tool-reference.md)を参照してください。

現在は schema 7、32 個の MCP ツール、FTS5 と短い CJK 文字列の厳密なフォールバック検索を提供します。実行ファイル名は `agent_llm_mm` のままです。

技術 MVP であり、正式な Local Alpha や本番品質を保証するものではありません。検索は字面一致で、意味検索ではありません。JSON バイト数はモデルのトークン数ではなく、namespace・呼び出し側の予算・フィードバックのラベルは認証ではありません。経験候補の有効化は検索対象を変えるだけで、手順の実行や権限の付与は行いません。export はバックアップではありません。

[現在の状態と検証範囲](project-status.md)および[唯一の active plan](plans/2026-07-10-product-replan.md)を確認してください。過去のテスト件数や計画を現在の状態と混同しないでください。

Bounded global identity/commitment versioning and explicit same-scope compensation: [contract](self-model-versions.md). Historical migration effective time stays unknown; this remains an experimental local MVP.
