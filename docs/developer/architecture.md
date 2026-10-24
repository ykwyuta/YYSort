# YYSortの現行処理パイプライン

| モジュール | 責務 |
| --- | --- |
| [main.rs](../../src/main.rs) | CLI・監督プロセス・制約付きワーカー起動 |
| [cards.rs](../../src/cards.rs) | 制御文・条件式・変形指定の解析と検証 |
| [records.rs](../../src/records.rs) | 固定長とRFC形式CSVのストリーム |
| [encoding.rs](../../src/encoding.rs) | YYLibraryの文字コードAPIとの接続 |
| [keys.rs](../../src/keys.rs) | CH、BI/FI、PD/ZD、CSV十進数などの比較・集計 |
| [engine.rs](../../src/engine.rs) | 安定ソート、ラン退避、多段マージ、集計 |
| [routing.rs](../../src/routing.rs) | OUTFILの条件・変形・分割 |
| [outputs.rs](../../src/outputs.rs) | 出力ステージング、公開・復旧、既存データの保護 |
| [resources.rs](../../src/resources.rs) | Windows Job Object／Linux rlimit・affinityと計測 |

処理順は入力→SKIPREC→INCLUDE/OMIT→STOPAFT→INREC→SORT/MERGE/COPY→SUM→OUTREC→OUTFIL→出力。
固定長の非キー部分は不透明なバイトとして保持します。CSVは文字コードを逐次変換し、論理レコードで処理します。
同値キーの入力順を保持し、退避と多段マージ後も安定性を検証します。

メモリに収まる区画でキーを計算し、専用Rayonプールでソートしてランを退避します。
ランは段階的にまとめ、ファンインと最大レコードを考慮した予算内でマージします。
OSのメモリ上限とチャンクの計上予算は異なり、RSS上限とは説明しません。

出力は各保存先へステージングし、通常エラー時は既存ファイルを保護します。
複数ファイルの一括原子性・電源断後の自動復旧は保証しません。
公開・復旧の境界条件は[単体実装](../../src/outputs.rs)、全CLIの不正入力・出力保全は[統合試験](../../tests/commands.rs)を参照してください。

未対応機能の要求・進捗は[初期提案](../history/proposals/initial-design.md)に分離します。
