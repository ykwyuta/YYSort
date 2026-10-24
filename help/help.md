# yysort 0.1

Windows/Linux用のCLI専用外部ソートです。DFSORT制御文の一部を実装します。
完全互換ではありません。`--help` はオプション、`--manual` はこのヘルプです。

## 実行例

```text
yysort input.dat -o output.dat --record-length 80 --key 1:10:CH:A --key 11:4:BI:D --memory 512MiB --threads 2 --stats
yysort input.csv -o output.csv --format csv --header --key 1:TEXT:A --key 3:NUM:D --encoding CP932
yysort part1.dat part2.dat -o merged.dat --record-length 80 --key 1:8:CH:A --merge --unique
yysort input.dat -o total.dat --record-length 80 --key 1:8:CH:A --sum 9:6:PD
yysort input.csv -o total.csv --format csv --header --key 1:TEXT:A --sum 3:NUM
yysort input.dat -o result.dat --record-length 80 --encoding IBM-939 --control sort.ctl
```

入力は通常ファイルを1個以上指定します。標準入力・標準出力へのレコード配信は未対応です。
出力は `-o` または `--out NAME=path` で指定します。出力が存在するときは `--overwrite` が必要です。入力・制御文・対応表のパスを出力には使えません。
同値キーは入力ファイル指定順・元レコード順を保持します（EQUALS相当）。
`--unique` は同値キーの先頭だけを保持します。`--sum` 自体が同値キーを1件に集約します。
MERGEは各入力が指定キー順であることを検証し、逆転したらエラーにします。
数値は浮動小数点へ変換せず、不正値・桁あふれはエラーです。

## 巨大ファイルとリソース上限

入力全体をメモリに載せません。予算内の区画を並列ソートしてランファイルへ退避し、
ヒープを使って多段マージします。ラン数が増えるとオンラインでまとめるため、開くファイル数と
メモリ上のラン一覧も制限されます。COPYは逐次処理、MERGEも検証付きランを作ってマージします。

| オプション | 意味 |
|---|---|
| `--memory 512MiB` | ワーカーに対するOS強制上限、最小64 MiB。ソート区画には原則1/4を予算として割り当て、残りはキー・デコーダー・マージ・ランタイム等に利用 |
| `--threads 2` | ソートワーカー数かつ利用可能な論理CPU数の上限。プロセスのCPUアフィニティも限定 |
| `--cpu-seconds N` | ワーカーの累積CPU時間の上限。Linuxはユーザー＋システム時間、Windowsはユーザー時間。経過時間の上限ではない |
| `--max-record 64KiB` | 1固定長レコード、復号後CSVの1論理レコード、BUILD結果の上限。最大16 MiB。列管理情報とキーにも余裕が必要 |
| `--merge-fan-in 32` | 1回のマージで読むラン数の上限（2〜128）。メモリと最大レコードに応じ、さらに減らす |
| `--temp-limit 64GiB` | 同時に存在するランファイルの合計。マージ中の入力ランと新ランを含む。最終出力のステージングは含まない |
| `--temp-dir DIR` | 作業ディレクトリの作成先。省略時は出力と同じ場所。場所は事前に作成 |
| `--stats` | 件数・ラン数・マージ回数・一時容量ピーク・管理上の区画メモリ・時間と、OSから取得したCPU数・メモリ上限・ピーク常駐メモリをstderrへJSON出力 |

サイズは整数とB/KiB/MiB/GiB/TiBで指定します。Linuxは `RLIMIT_AS` による**仮想アドレス空間**、
WindowsはJob Objectによる**コミットされた仮想メモリ**を制限します。RSSそのものの上限ではなく、
同じ数値でも両OSで意味は異なります。共有ライブラリ、スレッドスタック、アロケーターの予約領域も
影響し、予算が小さすぎると開始できない場合があります。設定できなければ無制限で続けずエラーです。
監督用の小さな親プロセスとOSページキャッシュはワーカー上限には含みません。

CPUは元の許可CPUから指定数を選びます。既定は利用可能数と2の小さい方です。許可数を超える指定はエラーです。
Windowsは現在のプロセッサーグループの範囲内です。CPU使用率のパーセント制限や他アプリへのCPU予約ではありません。

親プロセスが作業領域と出力ステージングを所有し、不正データ・上限・取消でワーカーが失敗したら
公開せず削除します。正常終了後に入力・制御文・対応表のパス・長さ・更新日時を検証して出力を置き換えます。
入力は処理中に変更しないでください。同じ長さ・日時に戻した変更まで検出するハッシュ検証や
スナップショットは行いません。Ctrl+Cで取消できます。親自身の強制終了や電源断まで一時ファイルの削除は保証しません。
出力ディスクには出力全体のステージング用空き容量も必要です。

## 固定長とキー

`--record-length` は改行・RDWを含まないバイト長で必須です。最後に不足レコードがあればエラーです。
位置は1始まりの**バイト位置**です。非キー部分を復号せず保持するため、パック十進数・NUL・バイナリーが混在しても保持します。

| 形式 | 意味 |
|---|---|
| CH | 生バイトの符号なし辞書順。EBCDICはEBCDICのバイト照合順 |
| TEXT | 指定文字コードで復号したUTF-8バイトの辞書順。ロケール照合やIBM UCA互換ではない |
| BI | ビッグエンディアン符号なし整数 |
| FI | ビッグエンディアン2の補数符号付き整数 |
| PD | 符号付きパック十進数。符号A/C/E/Fは正、B/Dは負 |
| ZD | EBCDIC符号付きゾーン十進数。途中のゾーンはF、最終ゾーンはA〜F |

キーは `位置:長さ:形式:A`、末尾Dで降順です。複数指定できます。TEXTは自己完結する文字列として
指定してください。DBCSの途中やSO/SI状態の途中で切ったキーの状態を、別の項目からは引き継ぎません。
固定長出力は元バイトを保持し、SUM/BUILD/OVERLAY指定部分だけ変更します。別の出力文字コードへの変換は未対応です。
SUM対象はキーや他のSUM対象と重ねられません。出力のPD/ZD符号は正C・負Dに正規化します。PD/ZDは格納された整数として集計し、小数位置を推定しません。

## CSVと文字コード

RFC 4180のカンマ、引用符、`""` の引用符エスケープ、引用符内のCR/LF、最終改行の省略に対応します。
不正な引用符・列数の不一致・不正な文字コード・規格にない制御文字はエラーです。
既定のレコード区切りはCRLFで、LF入力には `--allow-lf` を指定します。空のレコードは空の1フィールドです。
出力はCRLFと適切な引用符で再構成するため、元の引用符の付け方は保存しません。文字集合はUTF-8以外にも対応する拡張です。
`--header` は各入力の先頭を処理対象から外して1件だけ出力します。複数入力ではヘッダーの一致が必要です。
キーは `列番号:TEXT:A` / `列番号:NUM:D`（1始まり）。CH/TEXTは復号後UTF-8の辞書順、NUMは任意精度十進数です。
NUMは符号・小数点と前後の空白を扱い、指数・桁区切り・空欄のゼロ扱いはしません。
CSVの制御文フィールドは `列番号,1,TEXT|CH|NUM`、長さ1に限定します。BUILD/OVERLAYは固定長専用です。

文字コードはyyeditorの `yy-encoding` を直接利用します。UTF-8、UTF-16/32 LE/BE、Shift_JIS、CP932、
Shift_JIS-2004、EUC-JP、EUC-JIS-2004、ISO-2022-JP、IBM-037/500/1047/290/1027/930/939/1390/1399、
encoding_rsの欧州・中国語・韓国語等に対応します。既定はUTF-8で、自動推測はしません。
CSVの先頭BOMは指定文字コードで復号後に除去し、固定長のBOMはレコードデータです。
yyeditorの外部 `.map` も `--mapping path.map`（繰り返し可）で登録し、対応表名で指定できます。
対応表はUTF-8、1ファイル16 MiBまでです。CH・非キー部分の不正文字は復号しませんが、TEXT/CSVの不正文字は置換せずエラーです。

## DFSORT制御文の対応範囲

`--control` はUTF-8・最大1 MiBの自由書式です。複数行の括弧継続と行頭 `*` コメント、C'文字定数'、X'16進バイト列'を扱います。
C定数中の単引用符は2個重ねます。条件の再帰・連鎖は64までです。JCL/DD、72桁継続・ラベル・行末コメントは未対応です。
CLIのキー・COPY/MERGE・重複排除/集計オプションと制御文の併用はしません。

```text
 INCLUDE COND=(1,1,CH,NE,C'X',AND,(9,4,BI,GE,100,OR,9,4,BI,EQ,0))
 INREC BUILD=(1,8,9,4)
 SORT FIELDS=(1,8,CH,A,9,4,BI,D)
 SUM FIELDS=(9,4,BI)
 OUTREC BUILD=(C'RESULT:',1,12)
```

| 文 | 対応 |
|---|---|
| SORT / MERGE | FIELDS=(位置,長さ,形式,A/D,...)、FIELDS=COPY |
| OPTION | COPY、EQUALS、SKIPREC=n、STOPAFT=n |
| INCLUDE / OMIT | COND、ALL/NONE、FORMAT=形式、EQ/NE/LT/LE/GT/GE、定数・項目同士の比較、AND/OR・括弧。ANDを優先 |
| INREC / OUTREC | BUILD/FIELDS、OVERLAY。位置,長さ、C/X定数、出力列c:、nX/Xによる空白 |
| SUM | FIELDS=NONE、FIELDS=(位置,長さ,BI/FI/PD/ZD,...)。CSV拡張はNUM |
| OUTFIL | FNAMES、STARTREC/ENDREC、SAMPLE、ACCEPT、INCLUDE/OMIT、SAVE、BUILD/OUTREC/OVERLAY、SPLIT/SPLITBY/SPLIT1R |
| END | 引数なし |

処理順はSKIPREC → INCLUDE/OMIT → STOPAFT → INREC → SORT/MERGE → SUM → OUTREC → OUTFILです。
複数入力は指定順で数え、STOPAFTは抽出後に採用された件数です。ヘッダーは件数に含めません。
固定長CH条件の定数は項目と同じバイト長に限定し、暗黙の空白埋めはしません。
SUMは同値キーの先頭レコードを土台にします。未対応の文・オペランドはエラーです。

### 項目同士の比較・FORMAT

```text
 INCLUDE COND=(1,2,BI,GT,3,2,FI)
 INCLUDE FORMAT=BI,COND=(1,2,GT,3,2)
 INCLUDE COND=(1,2,GT,3,2),FORMAT=BI
```

各行は別の指定例です。右辺を位置,長さ,形式で指定すると同じレコード内の項目と比較します。
数値はBI/FI/PD/ZDを混在させ、桁数の異なる項目も整数値で比較できます。CSVではNUM同士を任意精度小数で比較します。
文字はCH同士またはTEXT同士を比較し、固定長CHの指定長は左右同じ長さに限定します。
文字と数値の混在、CHとTEXTの混在、長さの異なるCH比較に対するDFSORTの補正規則は未対応で、エラーにします。

`FORMAT=f`はINCLUDE/OMIT文全体の省略形式で、CONDの前後どちらにも指定できます。
明記した項目の形式が優先され、左辺・右辺それぞれに適用します。FORMATなしで形式を省略することはできません。
OUTFIL INCLUDE/OMITでも項目同士の比較ができますが、形式は左右それぞれに明記してください。
CSVでは例として`INCLUDE FORMAT=NUM,COND=(2,1,GT,3,1)`で2列目と3列目を比較できます。
短いレコードや不正な数値は従来どおりエラーです。VLSCMP/VLSHRTや日付等の条件形式は追加していません。

### 出力列指定・OVERLAY・空白

```text
 INREC OVERLAY=(9:C'OK',13:1,4)
 OUTREC BUILD=(1,8,12:C'END',2X)
```

`c:`は1始まりの出力バイト位置です。最初の指定を省略すると1、以降は直前の項目の続きに置きます。
BUILD/FIELDSは入力レコードから新しいレコードを作り、c:は前進のみで重複・後退はエラーです。
OVERLAYは指定範囲以外を保持し、必要ならレコードを拡張します。c:の後退と重複も扱います。
OVERLAYの項目は左から適用し、後の項目は前の項目で書き換えられた現在のレコードを参照します。
たとえばabcdefに`OVERLAY=(3:1,2,1:3,2)`を適用するとababefになります。
1項目内でのコピー元とコピー先の重なりも扱います。

未使用の拡張領域とBUILDで飛ばした領域は、入力文字コードの空白で埋めます。`X`は1バイト、`nX`はnバイトの空白です。
IBM-037等のEBCDICではX'40'、UTF-8/CP932等ではX'20'です。空白が複数バイトになる文字コードでは、
空白埋めを必要とする指定をエラーにします。変形の結果・指定位置の計算は`--max-record`以内に制限し、巨大な空白領域を先に確保しません。
INRECは抽出後・ソート前、OUTRECはSUM後、OUTFIL OVERLAYはグループの抽出後に適用します。
INREC/OUTRECそれぞれでBUILD/FIELDS/OVERLAYの併記はできず、OUTFILでもBUILD/OUTREC/OVERLAYは排他的です。
SEQNUM、数値変換・編集、IFTHEN、可変長入力は引き続き未対応です。

主な未対応: **OUTFILの帳票・HEADER/TRAILER等、JOINKEYS/JOIN/REFORMAT、IFTHEN/PARSE/FINDREP/SEQNUM、
ALTSEQ・UCA、SYMNAMES、ICETOOL、可変長VB/RDW、全数値編集・日付・浮動小数点、JCL/DD、ユーザー出口**。
対応拡大は提案書 `docs/design.md` に記載します。

## OUTFIL：複数出力・抽出・分割

制御文のDD名を `--out NAME=path` で実ファイルへ割り当てます。`-o path` はSORTOUTの割り当てです。
名前は英数字と@/#/$、先頭は数字以外、最大8文字で大文字小文字を区別しません。最大64出力・64グループです。
同じDD名を複数のOUTFILグループへ割り当てたり、複数DDを同じ出力パスへ割り当てたりすることはできません。
未割り当てDDや、参照されない名前の `--out` はエラーです。

```text
 SORT FIELDS=(1,4,CH,A)
 OUTFIL FNAMES=ACCEPT,INCLUDE=(5,1,CH,EQ,C'Y')
 OUTFIL FNAMES=REST,SAVE
```

```text
yysort input.dat --record-length 80 --control split.ctl --out ACCEPT=accepted.dat --out REST=rest.dat --memory 128MiB --threads 2
```

`-o` を併用すると、SORTOUTには全件、名前付きOUTFILには抽出結果を書きます。
FNAMESを省略したOUTFIL、またはFNAMES=SORTOUTはSORTOUT自体を制御するため、全件を二重には書きません。
OUTFILが複数あると、同じレコードが複数グループで採用されることもあります。

| パラメーター | 意味 |
|---|---|
| `FNAMES=A` / `FNAMES=(A,B)` | 出力先DD名。複数の場合は同じレコードを各出力へ複製。省略時SORTOUT |
| `STARTREC=n` / `ENDREC=m` | グローバルOUTREC後の1始まりの相対レコード位置。SUMで集約した後に数える |
| `SAMPLE=n` / `SAMPLE=(n,m)` | STARTRECからn件ごとの先頭m件を採用。n>=2、1<=m<=n。m省略時1 |
| `INCLUDE=(条件)` / `OMIT=(条件)` | 現在対応するCONDと同じ条件式、ALL/NONEにも対応。両方の併記はしない |
| `SAVE` | ほかのSAVEでない全グループで採用されなかったレコード。制御文の順序には依存しない。INCLUDE/OMITと併記不可 |
| `ACCEPT=n` | 範囲・サンプル・条件に合った先頭n件まで。ENDRECとは別の件数 |
| `BUILD=(...)` / `OUTREC=(...)` / `OVERLAY=(...)` | 固定長のグループ別変形。出力列指定・nXにも対応。グローバルOUTREC後の位置を参照。3つは排他的。CSVでは未対応 |
| `SPLIT` / `SPLITBY=n` | 採用・変形後のレコードをFNAMESの各出力へn件ずつ巡回。SPLITはn=1 |
| `SPLIT1R=n` | 各出力へn件ずつ順に渡し、余りは最後の出力へ渡す |

処理は範囲・サンプル → 条件またはSAVE → ACCEPT → グループ別BUILD → 分割 → 書き込みです。
SAVEはほかのグループのACCEPT上限等も考慮します。暗黙のSORTOUTへの全件出力はSAVEの除外条件に数えません。
CSVの `--header` は各出力へヘッダーを1回ずつ書き、範囲・採用件数・分割の件数には含めません。
`--stats` の `output_files` はDD名ごとのデータ件数です。`output_records` はグローバル処理後のストリーム件数で、複製数の合計ではありません。

全出力をそれぞれの出力先ディレクトリ内へステージングします。データ処理エラーやワーカーの上限終了では公開しません。
公開時は既存出力を同じディレクトリの `.yysort-backup-*` へ退避し、置き換え途中の通常エラーでは公開済みファイルを戻します。
復旧できない場合は原本のバックアップを削除せず、場所をstderrに表示します。出力先は同時に他のプログラムで変更しないでください。
複数ファイル全体をOSの1操作で原子的に置き換えるものではなく、公開中の途中状態は他プロセスから見えます。
親プロセスの強制終了・電源断まで自動復旧は保証しません。`--temp-limit` はラン専用で、全出力のステージング容量は別途必要です。
帳票、HEADER/TRAILER、FILES、FTOV/VTOF、REPEATなど、表にないOUTFILパラメーターは引き続き未対応です。

## 終了コード・検証

0は成功、2は設定・データ等のエラー、130は捕捉した取消です。OSが上限でワーカーを終了した場合は
ワーカーの非ゼロ終了を返すため、必ずしも2ではありません。stdoutにデータは出しません。
`demo/benchmark.py` はメモリより大きいファイルを生成し、全件の元データ・順序・安定性・重複欠落なしを逐次検証します。
速度はディスク・形式・キー数・設定で変わります。実測をDFSORTと同条件の速度保証とは扱いません。

## 公式仕様

- IBM [制御文](https://www.ibm.com/docs/en/zos/3.2.0?topic=jobs-summary-dfsort-control-statements)
- IBM [SORT](https://www.ibm.com/docs/en/zos/2.5.0?topic=statements-sort-control-statement)
- IBM [OUTFIL](https://www.ibm.com/docs/en/zos/3.2.0?topic=statements-outfil-control)
- IBM [処理順](https://www.ibm.com/docs/en/zos/3.2.0?topic=works-dfsort-processing)
- IBM [INREC・OVERLAY](https://www.ibm.com/docs/en/zos/3.2.0?topic=statements-inrec-control-statement)
- IBM [OVERLAYによる拡張](https://www.ibm.com/docs/en/zos/3.1.0?topic=fields-extending-records-overlay)
- IBM [INCLUDE・FORMAT・項目間比較](https://www.ibm.com/docs/en/zos/3.2.0?topic=statements-include-control-statement)
- [RFC 4180](https://www.rfc-editor.org/rfc/rfc4180)
- Microsoft [Job Object memory limits](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_extended_limit_information)
- Linux [setrlimit](https://man7.org/linux/man-pages/man2/getrlimit.2.html)
