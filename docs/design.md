# yysort：資源制約付きの大規模レコード処理

## 要求と到達目標

Windows/Linuxで動作するCLI専用ツールを追加する。IBM DFSORTが提供する
ソート、マージ、コピー、抽出、集計、変形、結合、複数出力・帳票処理を対応目標とする。
固定長とRFC 4180 CSVを扱い、yyeditorと同じ文字コードを利用する。
巨大ファイルを入力全体のメモリ展開なしに処理し、CPUとメモリの上限を利用者が指定できることを必須とする。

DFSORTは制御文・オペランド・数値形式が広く、初版を完全互換とは呼ばない。
互換性は制御文単位だけでなくオペランド、入力形式、照合順、短い項目、数値エラー等も含めて管理する。
未対応指定を黙って無視したり、推測で置き換えて成功扱いにしない。

## 構成と再利用

- リポジトリルート：独立Cargoワークスペース。GUI、Windows UI部品、外部ソートプログラムには依存しない。
- YYLibraryの`yy-encoding`：コミットを固定したCargo Git依存。文字コード・対応表と検証の本体はYYLibraryで管理する。
- `src/cards.rs`：DFSORTの制御文を実行計画へ変換。自由書式と互換範囲を明示する。
- `src/records.rs`：固定長／厳密CSVのストリーミング読み書き。既存のCSV表示用パーサーは寛容なため、不正入力拒否が必要なバッチ処理にはそのまま流用しない。
- `src/keys.rs`：符号なし・符号付きバイナリー、PD/ZD、任意精度十進数、文字キー。
- `src/engine.rs`：ラン生成、オンライン圧縮、多段マージ、抽出・変形・集計。
- `src/resources.rs`：Windows/LinuxのCPUアフィニティ、メモリ上限、累積CPU時間上限、使用量取得。
- `.github/workflows/yysort.yml`：Windows/Linuxの独立検査、実プロセス試験、大容量検証、配布用ライセンス収集。

CLIのオフラインヘルプは `--manual` で同梱Markdownを表示する。
既存YYアプリ同様にソースとヘルプを一緒に更新し、外部サイトなしで仕様・制約を読めるようにする。

## 処理と資源モデル

入力 → SKIPREC → INCLUDE/OMIT → STOPAFT → INREC → SORT/MERGE/COPY → SUM → OUTREC → OUTFIL → 出力。
同値キーには入力ファイル順・レコード順の64ビット通番を使い、退避や複数段のマージを経ても順序を保持する。
固定長の非キー部分を文字として読む必要はなく、パック十進数や任意のバイトを保持する。
CSVは文字コードを逐次復号し、引用符内改行を含む論理レコードとして読む。

メモリ予算の一部で区画を作り、キーを前計算して専用Rayonプールでソートし、ランファイルへ退避する。
ラン一覧を入力件数に比例して増やさず、同じレベルのランが一定数になったらオンラインでマージする。
レベル数は対数的に増える。マージは最大ファンインと最大レコードに応じたメモリ予算を守り、ヒープを使う。
キー・レコード・CSV列情報の容量も区画計算に含める。最大レコードは明示し、予算外の巨大な1レコードを許容しない。

`--threads` は専用ワーカー数とCPUアフィニティで実際の論理コア数を制限する。
CPU使用率のパーセント指定とは区別する。`--cpu-seconds` は累積CPU時間の上限であり、経過時間ではない。
メモリは、LinuxのRLIMIT_AS（仮想アドレス空間）、WindowsのJob Object（コミットメモリ）で強制する。
RSS、コミット、仮想空間は同一の指標として表示しない。設定に失敗した場合は処理を拒否する。

監督プロセスは別プロセスのワーカーを起動し、作業領域と出力ステージングを所有する。
ワーカーがOOMやCPU上限で終了しても出力を公開せず、一時領域を回収する。
正常終了と入力メタデータ検証後に出力を置き換える。初版は通常ファイルを対象とし、ストリーム出力や再開可能なジョブは別途実装する。
監督プロセス、OSページキャッシュ、出力ステージングのディスク容量はワーカーのメモリ／ラン容量上限とは別である。

## 互換性の段階

| 区分 | 初版の実装 | 残る対応目標 |
|---|---|---|
| 並べ替え | SORT/MERGE FIELDS、COPY、複数キー、A/D、安定順序 | 追加FORMAT、VLSHRT等、ALTSEQ、ロケール／UCA |
| 型 | CH、BI、FI、PD、ZD、拡張TEXT/NUM | DFSORTの残りの数値形式、浮動小数点、編集形式、日付 |
| 抽出 | INCLUDE/OMIT、定数・項目間比較、FORMAT省略指定、AND/OR・括弧 | 不等長CH補正、部分文字列、ビット・日付条件等 |
| 変形 | INREC/OUTREC BUILD/FIELDS/OVERLAY、C/X定数、出力列c:、nX空白 | IFTHEN、PARSE、FINDREP、編集・変換、連番 |
| 集計 | SUM FIELDS=NONE、BI/FI/PD/ZD、CSV NUM | その他の形式、追加オペランド、互換エラー処理の拡充 |
| 複数出力 | OUTFIL FNAMES、INCLUDE/OMIT/SAVE、範囲・サンプル・ACCEPT、BUILD/OVERLAY、SPLIT/SPLITBY/SPLIT1R | ヘッダー／トレーラー、帳票、その他のOUTFILオペランド |
| 結合 | 未実装 | JOINKEYS/JOIN/REFORMAT。重複群をディスクへ退避し巨大な直積でもメモリ上限を維持 |
| 補助 | OPTION COPY/EQUALS/SKIPREC/STOPAFT、END | SYMNAMES、追加OPTION、ICETOOL操作 |
| 入力 | 固定長バイト列、RFC 4180 CSV拡張 | VB/RDW等、レコード変換、標準入出力 |
| ホスト連携 | ファイルパス等をCLIで指定 | JCL/DDの移行アダプター。z/OSの製品実行環境・ユーザー出口は未実装 |

現行の詳細文法と制約は [同梱ヘルプ](../help/help.md) を正とする。
表の未実装部分は未完了要件として残し、完全互換や「全DFSORTジョブが動く」とは案内しない。

## 検証と性能

Windows/Linux双方で同じCLI試験を実プロセスとして実行する。
複数キー・安定性・全バイト保持・CSV引用符と改行・文字コード変換・外部対応表・数値の符号と桁あふれ・
抽出と変形の処理順・入力順検証・上限・失敗時出力保護を確認する。
外部退避と多段マージを必ず起こし、OSに問い合わせてCPU・メモリ上限を確認する。

`demo/benchmark.py` は入力を生成し、全出力レコードの内容・順序・安定性・重複欠落を逐次検証する。
メモリ上限より大きい512 MiB以上を両OSで測り、2 GiB以上も検証する。
入力サイズ、件数、CPU数、上限の指標、ピークRSS、実行時間、ラン数、一時容量、検証結果を記録する。
結果をDFSORTと同条件の優劣や全形式の性能保証にはしない。
より大きなデータ、CSVの長大項目、多数キー、PD/DBCS、低速ディスク、ディスク枯渇、取消、権限制約も継続評価する。

## 公式仕様

- IBM [DFSORT control statements](https://www.ibm.com/docs/en/zos/3.2.0?topic=jobs-summary-dfsort-control-statements)
- IBM [SORT control statement](https://www.ibm.com/docs/en/zos/2.5.0?topic=statements-sort-control-statement)
- IBM [DFSORT processing](https://www.ibm.com/docs/en/zos/3.2.0?topic=works-dfsort-processing)
- [RFC 4180](https://www.rfc-editor.org/rfc/rfc4180)
- Microsoft [Job Object memory limits](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_extended_limit_information)
- Linux [getrlimit / setrlimit](https://man7.org/linux/man-pages/man2/getrlimit.2.html)
