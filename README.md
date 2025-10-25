# tcp-traffic-scan

TCP 帯域推測ツール with Prometheus Metrics Export

## 概要

このツールは、指定されたネットワークインターフェースを使用して、複数のサーバーへの TCP 接続を確立し、RTT（Round Trip Time）と受信バッファサイズから帯域幅を推測します。推測された帯域幅は Prometheus メトリクスとして公開され、HTTP エンドポイント経由で補正値を適用できます。

## 機能

- 複数のネットワークインターフェース（eth0, eth1 など）を指定可能
- 複数のサーバー（1.1.1.1, 8.8.8.8 など）に対して同時測定
- **Prometheus メトリクス出力**（ポート 59121）
- **HTTP API による補正値の設定**（ポート 32600）
- リアルタイムでの帯域幅推測

## インストール

```bash
cargo build --release
```

## 使用方法

### 基本的な使い方

```bash
./run.sh -i eth0 -i eth1 -s 1.1.1.1 -s 1.0.0.1 -s 8.8.8.8 -s 8.8.4.4
```

または直接実行:

```bash
cargo run -- -i eth0 -i eth1 -s 1.1.1.1 -s 8.8.8.8
```

### オプション

- `-i, --interface <INTERFACE>`: 測定に使用するネットワークインターフェース（複数指定可能）
- `-s, --server <SERVER>`: 測定対象サーバーの IP アドレスまたはホスト名（複数指定可能）

## Prometheus メトリクス

### メトリクスエンドポイント

```
http://localhost:59121/metrics
```

### メトリクス形式

全てのメトリクスは **bps（bits per second）** 単位で出力されます。

```
# 各サーバーIPごとの帯域幅
tcp_traffic_scan_tcp_bandwidth_bps{interface="eth0",server_ip="1.1.1.1"} 150500000
tcp_traffic_scan_tcp_bandwidth_bps{interface="eth0",server_ip="8.8.8.8"} 200300000
tcp_traffic_scan_tcp_bandwidth_bps{interface="eth1",server_ip="1.1.1.1"} 180200000
tcp_traffic_scan_tcp_bandwidth_bps{interface="eth1",server_ip="8.8.8.8"} 220700000

# 各インターフェースごとの平均帯域幅
tcp_traffic_scan_tcp_bandwidth_avg_bps{interface="eth0"} 175400000
tcp_traffic_scan_tcp_bandwidth_avg_bps{interface="eth1"} 200450000
```

### Prometheus 設定例

`prometheus.yaml`:

```yaml
global:
  scrape_interval: 1s
  evaluation_interval: 1s

scrape_configs:
  - job_name: "tcp-traffic-scan"
    scrape_interval: 1s
    static_configs:
      - targets: ["localhost:59121"]
```

## HTTP 補正値 API

測定値に補正係数を適用できます。インターフェースごとに個別の補正値を設定することも、全体のデフォルト補正値を設定することもできます。

### インターフェース別の補正値を設定

wan0 は eth0 に、wan1 は eth1 にマッピングされます。

```bash
# wan0（eth0）の測定値を10倍にする
curl "http://localhost:32600/tcpflow?value=10&nic=wan0"

# wan1（eth1）の測定値を5倍にする
curl "http://localhost:32600/tcpflow?value=5&nic=wan1"

# 特定のインターフェース（eth0）の測定値を2倍にする
curl "http://localhost:32600/tcpflow?value=2&nic=eth0"
```

### デフォルト補正値を設定（全インターフェース）

nic パラメータを指定しない場合、全インターフェースのデフォルト補正値が設定されます。

```bash
# 全ての測定値を3倍にする
curl "http://localhost:32600/tcpflow?value=3"
```

### 現在の補正値を確認

```bash
curl "http://localhost:32600/tcpflow"
```

レスポンス例:

```
Default correction factor: 1
```

## 実行例

```bash
$ cargo run -- -i eth0 -i eth1 -s 1.1.1.1 -s 8.8.8.8

Prometheus metrics available at http://localhost:59121/metrics
Correction factor API available at http://localhost:32600/tcpflow?value=<factor>
Starting measurements...
==================================
eth0: |1.1.1.1:150500000bps|8.8.8.8:200300000bps|avg:175400000bps|
eth1: |1.1.1.1:180200000bps|8.8.8.8:220700000bps|avg:200450000bps|
eth0: |1.1.1.1:152000000bps|8.8.8.8:198000000bps|avg:175000000bps|
eth1: |1.1.1.1:179000000bps|8.8.8.8:222000000bps|avg:200500000bps|
...
```

別のターミナルで:

```bash
# メトリクスを確認
$ curl http://localhost:59121/metrics
# HELP tcp_traffic_scan_tcp_bandwidth_bps TCP bandwidth estimation in bps
# TYPE tcp_traffic_scan_tcp_bandwidth_bps gauge
tcp_traffic_scan_tcp_bandwidth_bps{interface="eth0",server_ip="1.1.1.1"} 150500000
tcp_traffic_scan_tcp_bandwidth_bps{interface="eth0",server_ip="8.8.8.8"} 200300000
tcp_traffic_scan_tcp_bandwidth_bps{interface="eth1",server_ip="1.1.1.1"} 180200000
tcp_traffic_scan_tcp_bandwidth_bps{interface="eth1",server_ip="8.8.8.8"} 220700000

# HELP tcp_traffic_scan_tcp_bandwidth_avg_bps TCP bandwidth average per interface in bps
# TYPE tcp_traffic_scan_tcp_bandwidth_avg_bps gauge
tcp_traffic_scan_tcp_bandwidth_avg_bps{interface="eth0"} 175400000
tcp_traffic_scan_tcp_bandwidth_avg_bps{interface="eth1"} 200450000
...

# wan0（eth0）の補正値を10倍に設定
$ curl "http://localhost:32600/tcpflow?value=10&nic=wan0"
Correction factor for wan0 (eth0) set to: 10

# wan1（eth1）の補正値を5倍に設定
$ curl "http://localhost:32600/tcpflow?value=5&nic=wan1"
Correction factor for wan1 (eth1) set to: 5

# 現在の補正値を確認
$ curl "http://localhost:32600/tcpflow"
Default correction factor: 1

Per-interface correction factors:
  eth0: 10
  eth1: 5

# 補正後のメトリクスを確認（eth0は10倍、eth1は5倍になる）
$ curl http://localhost:59121/metrics
tcp_traffic_scan_tcp_bandwidth_bps{interface="eth0",server_ip="1.1.1.1"} 1505000000
tcp_traffic_scan_tcp_bandwidth_bps{interface="eth0",server_ip="8.8.8.8"} 2003000000
tcp_traffic_scan_tcp_bandwidth_avg_bps{interface="eth0"} 1754000000

tcp_traffic_scan_tcp_bandwidth_bps{interface="eth1",server_ip="1.1.1.1"} 901000000
tcp_traffic_scan_tcp_bandwidth_bps{interface="eth1",server_ip="8.8.8.8"} 1103500000
tcp_traffic_scan_tcp_bandwidth_avg_bps{interface="eth1"} 1002250000
...
```

## 注意事項

### Linux

- インターフェースへのバインドには`SO_BINDTODEVICE`を使用します
- root 権限または`CAP_NET_RAW`ケーパビリティが必要な場合があります

### macOS

- `SO_BINDTODEVICE`はサポートされていないため、インターフェースの指定は無視されます
- OS が自動的にインターフェースを選択します

## 依存関係

- clap: コマンドライン引数パース
- socket2: ソケット操作
- libc: システムコール
- ctrlc: Ctrl+C ハンドリング
- tokio: 非同期ランタイム
- axum: HTTP サーバー
- prometheus: メトリクス出力
- lazy_static: グローバル変数管理
- serde: シリアライゼーション

## ライセンス

MIT
