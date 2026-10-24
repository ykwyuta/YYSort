# yysort

Windows/Linux用、CLI専用の固定長・RFC 4180 CSV外部ソートです。
メモリに収まらない入力をランへ退避して多段マージし、CPUアフィニティ・メモリ上限をOSで制限します。
[YYLibrary](https://github.com/ykwyuta/YYLibrary)の`yy-encoding`による日本語・EBCDIC・外部対応表を利用します。
独立Cargoワークスペース、独立CIです。GUIや外部ソートプログラムは不要です。

```text
cargo build --release --locked
yysort input.dat -o output.dat --record-length 80 --key 1:8:CH:A --memory 512MiB --threads 2 --stats
yysort input.csv -o output.csv --format csv --header --key 3:NUM:D --encoding CP932
yysort input.dat -o output.dat --record-length 80 --encoding IBM-939 --control sort.ctl
yysort --manual
```

安定ソート、検証付きMERGE、COPY、複数キー、重複排除、BI/FI/PD/ZD集計、CSVの任意精度十進数、
INCLUDE/OMIT（項目間比較・FORMAT指定）、INREC/OUTREC BUILD/FIELDS/OVERLAY、出力列指定・空白挿入、
OUTFILの複数出力・抽出・変形・サンプリング・分割を実装しています。**DFSORTの完全互換ではありません**。
OUTFIL帳票・JOINKEYS・ICETOOL等は未対応で、指定した未対応構文は無視せずエラーにします。

`--threads` は論理コア数、`--memory` はワーカーのLinux仮想アドレス空間／Windowsコミットメモリの上限です。
CPU使用率のパーセント指定やRSS自体の上限ではありません。詳細は [ヘルプ](help/help.md)、
全体の目標は [設計書](docs/design.md) を参照してください。
文字コードクレートはYYLibraryの固定コミットをCargoで自動取得します。
更新時はCargo.tomlの`rev`とCargo.lockを合わせて更新してください。

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked -- --test-threads=1
python demo/benchmark.py --exe target/release/yysort --mib 512 --memory 128MiB --threads 2 --directory target/yysort-benchmark
```

## 配布資材の作成

```text
python tools/package.py --test
```

Python 3.10以降を使用します。`dist/yysort/`に実行ファイル、オフラインヘルプ、
作例、設計書、ソース一式、GPL本文と例外条項、依存ライブラリのライセンスを出力します。
Windows/Linuxの独立CIは書式・Clippy・全試験・releaseビルド・256MiBの全件照合と
配布資材の作例実行を行います。[検証記録](VALIDATION.md)も参照してください。

配布用の`source/vendor/YYLibrary`には採用した共有ライブラリのソースを含め、
Cargo.tomlとCargo.lockは同じGitコミット参照を維持します。
