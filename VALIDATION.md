# YYSort 検証記録

2026-10-24、YYLibraryの固定コミットを利用する独立ワークスペースで検証。
ソート処理の実装とテストを維持し、文字コード処理はYYLibraryの独立CIで検証する。
ローカルパスによる元リポジトリへの依存はない。

| 検証 | Windows | Linux |
| --- | --- | --- |
| 環境 | Rust 1.92.0、Windows、Dドライブ | Rust 1.99.0、Debian Bookwormコンテナー |
| cargo fmt / Clippy（警告をエラー） | 成功 | 成功 |
| YYLibrary側の文字コード単体・統合試験 | 31件成功 | 31件成功 |
| ソート単体・実CLI試験 | 28件成功 | 29件成功 |
| releaseビルド | 成功 | 成功 |

CSVの引用符内改行、日本語/EBCDIC/外部対応表、PD/ZD/BI/FI、安定ソート、外部退避・多段マージ、
制御文・OVERLAY・OUTFIL、資源上限、エラー時の既存出力保全を含む。
Linuxでは非UTF-8ファイル名も検証する。Windowsのファイル復旧試験は通常権限で実施した。

## 大容量検証

256MiB、1,048,576件、256バイト固定長、ワーカーメモリ上限128MiB・2コア。
項目間比較、INREC/OUTREC OVERLAY、主出力とOUTFIL巡回分割2出力を対象に、
全件の内容・順序・安定性・重複欠落なしを逐次照合する。
初期ラン34・中間マージ1。Windowsは5.93秒、ピークRSS26,161,152バイト。
Linuxは1.48秒、ピークRSS22,405,120バイト。
測定は起動・出力公開込みで、入力生成・検証は除外する。OS間の速度比較や速度保証には使わない。

## 配布

`python tools/package.py --test`でCLI・オフラインヘルプ・作例・ソース一式・依存ライセンスを作成する。
配布された実行ファイルでCSV/固定長の作例と`--manual`を実行する。
GitHubでの検証結果は[Actions](https://github.com/ykwyuta/YYSort/actions)を参照。
以前の測定記録は[検証履歴](docs/validation-history.md)に保存している。
