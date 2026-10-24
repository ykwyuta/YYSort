# yysort 初版の検証記録（2026-10-24）

## DFSORT互換の追加：OVERLAY・出力列指定・項目間比較

2026-10-24、INREC/OUTRECのOVERLAY、INREC/OUTREC FIELDS別名、BUILD/OUTFILの出力列c:、
空白X/nX、OUTFIL OVERLAY、INCLUDE/OMITの項目間比較とFORMAT指定を追加した。
Windows（Rust 1.92.0）は28試験、Linux（Rust 1.99.0、既存検証用Dockerイメージ）は29試験が成功。
両OSでfmt、Clippy（workspace/all-targets、警告をエラー）とreleaseビルドを確認した。
LinuxはホストのCargo資材からオフラインで依存を供給し、ソースを読み取り専用にして実行した。

検証項目：前のOVERLAY結果を参照する連続上書き、単一項目内の重なり、領域拡張と空白、
EBCDICのX'40'空白、出力列指定、INREC→ソート→OUTREC→OUTFILの順序、複数出力の独立性、
CHの全比較演算子、BI/FIおよびPD/ZDの混合比較、FORMATの前置・後置と項目指定の優先、
CSVの任意精度数値の項目間比較、空入力でも不正指定・巨大位置を拒否して既存出力を保持すること。
既存の外部退避・多段マージ試験もINREC/OUTREC OVERLAYを組み合わせ、全レコードを照合した。

追加の性能検証は512MiB・2,097,152件・256バイト固定長、メモリ上限128MiB・2コア。
項目同士の比較とINREC/OUTREC OVERLAYを適用し、主出力と巡回分割2出力の全バイト・安定順序・欠落重複なしを照合した。

| 環境 | wall時間 | 速度 | ピークRSS | 初期ラン / 中間マージ |
|---|---:|---:|---:|---:|
| Windows、Dドライブ | 5.25秒 | 97.6MiB/s | 26,402,816バイト | 68 / 2 |
| Linux、Docker専用ボリューム | 3.41秒 | 150.0MiB/s | 22,659,072バイト | 68 / 2 |

チャンクの最大計上量は両OSとも33,553,600バイトで32MiB予算内。一時ラン最大817,305,600バイト。
Windowsはコミット、Linuxは仮想アドレス空間の上限であり、RSS上限と同一ではない。
入力生成・検証時間は除外し、起動・公開を含む。OS間比較・DFSORTとの性能比較には使用しない。
Windowsの記録は`target/yysort-dfsort-overlay-windows-512/result.json`、
Linuxの記録は専用ボリューム`yysort-validation-build`内の`/benchmark-linux-dfsort-overlay-512/result.json`。
後述する以前のDocker障害後のLinux再検証は、今回の64KiB出力バッファ版で成功している。
専用CIにも`--outfil --reformat`での全件検証を追加した。GitHub CIは未実行。
不等長CHの補正、IFTHEN/SEQNUM/JOINKEYS/帳票等は未対応で、IBM DFSORT実機との直接照合はしていない。

以下は追加前の測定・検証履歴。

WindowsホストとLinux Docker（rust:1.99.0-bookworm）でRust 1.99.0を使用。
両OSで fmt、Clippy `--workspace --all-targets --locked -- -D warnings`、12試験（単体1・実CLI11）、releaseビルドを確認しました。
ルートワークスペースもfmt・Clippy・metadata確認が成功しています。独立アプリの除外設定以外の共有Rust実装は変更していません。

試験は固定長の複数キー・安定性・不透明バイト保持、CSVの引用符内CRLFと二重引用符、任意精度小数集計、
CP932/Shift_JIS-2004/EUC-JIS-2004/UTF-16LE/ISO-2022-JP変換、外部EBCDIC対応表、BI/FI/PD/ZD集計、
制御文の処理順、MERGEの入力順検証、STOPAFT以降を読まないこと、破損入力・桁あふれ・未対応構文・上限での出力保護を対象にしました。
外部ソートの試験では強制的に退避・多段マージを起こし、実OSからCPUアフィニティ1コアとメモリ上限128 MiBを取得して確認しました。

## 固定長の大容量測定

256バイト、先頭8バイトをCHキー、16バイトの入力通番と固定ペイロードを持つ生成レコード。
同値キーを含めて全出力レコードを逐次検証し、元データ・通番・順序・安定性・重複欠落なしを確認しました。
入力生成と検証は処理時間から除外し、プロセス起動と最終公開はwall時間に含めています。
生成直後の入力を使い、OSキャッシュは無効化していません。
WindowsはDドライブ、LinuxはDockerボリュームなので、OS間の速度の優劣を示す比較ではありません。

| 環境 | 入力 | 件数 | 指定CPU | ワーカー上限 | wall時間 | 実測速度 | ピークRSS | 初期ラン / 中間マージ |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Windows | 512 MiB | 2,097,152 | 2 | 128 MiBコミット | 6.62秒 | 77.4 MiB/s | 約24.8 MiB | 68 / 2 |
| Windows | 2 GiB | 8,388,608 | 2 | 128 MiBコミット | 15.98秒 | 128.2 MiB/s | 約25.2 MiB | 269 / 8 |
| Linux Docker | 512 MiB | 2,097,152 | 2 | 128 MiB仮想空間 | 5.90秒 | 86.8 MiB/s | 約21.2 MiB | 68 / 2 |

Windows 2 GiBでは管理上の区画メモリが最大33,553,600バイト、一時ラン容量のピークが2,451,916,800バイトでした。
OSに問い合わせたアフィニティは2コア、上限は134,217,728バイトです。
RSS・コミット・仮想空間は別指標であり、OSキャッシュや監督プロセスをこの値へ合算していません。

元の記録は `target/yysort-benchmark-windows-{512,2048}/result.json`、
Linuxは `target/yysort-benchmark-linux-512.json` に保存しています。
生成用スクリプトは `demo/benchmark.py` です。これらを全形式の速度保証やDFSORTと同条件の比較とは扱いません。

Windowsでは同じ2 GiB入力に `--cpu-seconds 1 --threads 1` を適用してOSの時間上限で実際に終了させました。
非ゼロ終了 -1073741756を確認し、元の出力内容keepが保持され、専用作業領域とステージングが残らないことを確認しました。
LinuxのワーカーはCPU上限やOOM時のコアダンプによる無制限なディスク書込みを避けるため、RLIMIT_COREを0にします。

## CI・配布

独立ワークフロー `yysort.yml` を追加し、Windows/Linuxの検査・実プロセス試験・256 MiB性能検証と成果物作成を設定しました。
ワークフローYAMLの構文を検査しています。GitHub CIの起動・プッシュ・公開は行っていません。
解決済みCargo依存のうち実行対象OSで到達可能な依存をたどり、Windows63件・Linux53件のライセンス／NOTICEを収集しました。
CLIヘルプとデモの固定長・CSV実行も確認しました。

DFSORT完全互換、OUTFIL帳票/JOINKEYS/ICETOOL、VB/RDW、すべての数値形式、ホストJCLや出口プログラムは未実装です。
対応目標と現状の区別は `docs/history/proposals/initial-design.md` と同梱ヘルプに記載しています。


## OUTFIL拡張の検証（2026-10-24）

OUTFILのFNAMES、INCLUDE/OMIT/ALL/NONE、SAVE、STARTREC/ENDREC、SAMPLE、ACCEPT、
BUILD/OUTREC、SPLIT/SPLITBY/SPLIT1Rを追加しました。CLIの `--out NAME=path` で最大64出力を割り当てます。
SORTOUTの暗黙の全件出力と明示的OUTFIL制御を区別し、SAVEを制御文順序によらず全グループへ適用しています。
CSVの各出力へのヘッダーと引用符内改行、SUM・グローバルOUTREC後の位置と値、外部退避・多段マージ後の出力を確認しました。
STOPAFTは公式の処理順に合わせ、INCLUDE/OMIT後の採用件数に修正しました。初版の上記説明ではなく現在のヘルプを参照してください。

出力全体をステージングし、公開途中の通常エラーで既存ファイルへ戻す処理を追加しました。
途中まで新しい出力を公開した後の失敗、既存・新規出力の混在、外部編集を検知した際の原本バックアップ保持を試験しました。
外部編集やFS障害で復旧できない場合はバックアップを削除せず場所を診断します。複数ファイルの一括原子性や、強制終了時の自動復旧は保証しません。

Windowsの最終fmt・Clippy・21試験（単体5、実CLI16）とreleaseビルドが成功しました。
LinuxはClippy・22試験（単体5、実CLI17）とreleaseビルドが成功しました。
Linux専用試験は非UTF-8ファイル名を出力先へ指定して監督プロセスとワーカー間で維持できることも検証しています。

Windowsでは主出力と分割2出力を同時に生成する512 MiB検証を実施し、全レコード・内容・安定性・分割先を照合しました。
64 KiBバッファの最終版は128 MiBワーカー上限・2コアで4.73秒、約108.3 MiB/s、ピークRSS26,435,584バイトでした。
2,097,152レコードを主出力へ、1,048,576レコードずつを分割先へ書き、初期ラン68・中間マージ2でした。
記録は `target/yysort-outfil-windows-64k-512/result.json` です。
同じ構成に累積CPU時間1秒上限を適用すると非ゼロ終了し、3つの既存出力がすべて保持され、一時ファイルが回収されたことも確認しました。

Linuxの16 KiBバッファ版でも512 MiBの全件・分割先照合が成功しました（36.54秒、ピークRSS22,441,984バイト）。
書込み回数を減らすため64 KiBへ変更し、Linuxのreleaseビルドは成功しましたが、追加性能検証はI/Oエラーで中断しました。
Dockerがoverlay2の削除時にread-only file systemを返し、その後Docker Desktop is unable to startを返す状態になっています。
64 KiB版のLinux最終再検証は未完了です。測定値は生成直後のキャッシュ等を含む実環境の記録で、OS間比較・速度保証ではありません。

中断した検証用コンテナーはc081366d5f76bc8ce9848a753cf7db18dad6ee51430a5a2abde7613f9007b3ceです。
当該コンテナーのみへ停止要求を出しましたが、デーモンが起動不能のため最終状態の再取得はできていません。
ほかのDBコンテナーの停止・Dockerサービスの再起動・ボリューム削除は行っていません。
CIには通常の256 MiB検証に加えてOUTFIL分割と全件照合の256 MiB検証を設定しました。GitHub CI自体は起動していません。
