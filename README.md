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

```
tcp_traffic_scan_tcp_bandwidth_mbps{interface="eth0",server_ip="1.1.1.1"} 150.5
tcp_traffic_scan_tcp_bandwidth_mbps{interface="eth0",server_ip="8.8.8.8"} 200.3
tcp_traffic_scan_tcp_bandwidth_mbps{interface="eth1",server_ip="1.1.1.1"} 180.2
tcp_traffic_scan_tcp_bandwidth_mbps{interface="eth1",server_ip="8.8.8.8"} 220.7
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

測定値に補正係数を適用できます。全てのメトリクス値が指定された係数で乗算されます。

### 補正値を設定

```bash
# 測定値を10倍にする
curl "http://localhost:32600/tcpflow?value=10"

# 測定値を0.5倍にする（半分）
curl "http://localhost:32600/tcpflow?value=0.5"

# 測定値を2倍にする
curl "http://localhost:32600/tcpflow?value=2"
```

### 現在の補正値を確認

```bash
curl "http://localhost:32600/tcpflow"
```

レスポンス例:

```
Current correction factor: 10
```

## 実行例

```bash
$ cargo run -- -i eth0 -i eth1 -s 1.1.1.1 -s 8.8.8.8

Prometheus metrics available at http://localhost:59121/metrics
Correction factor API available at http://localhost:32600/tcpflow?value=<factor>
Starting measurements...
==================================
eth0: |1.1.1.1:150Mbps|8.8.8.8:200Mbps|
eth1: |1.1.1.1:180Mbps|8.8.8.8:220Mbps|
eth0: |1.1.1.1:152Mbps|8.8.8.8:198Mbps|
eth1: |1.1.1.1:179Mbps|8.8.8.8:222Mbps|
...
```

別のターミナルで:

```bash
# メトリクスを確認
$ curl http://localhost:59121/metrics
# HELP tcp_traffic_scan_tcp_bandwidth_mbps TCP bandwidth estimation in Mbps
# TYPE tcp_traffic_scan_tcp_bandwidth_mbps gauge
tcp_traffic_scan_tcp_bandwidth_mbps{interface="eth0",server_ip="1.1.1.1"} 150.5
tcp_traffic_scan_tcp_bandwidth_mbps{interface="eth0",server_ip="8.8.8.8"} 200.3
...

# 補正値を10倍に設定
$ curl "http://localhost:32600/tcpflow?value=10"
Correction factor set to: 10

# 補正後のメトリクスを確認（全ての値が10倍になる）
$ curl http://localhost:59121/metrics
tcp_traffic_scan_tcp_bandwidth_mbps{interface="eth0",server_ip="1.1.1.1"} 1505.0
tcp_traffic_scan_tcp_bandwidth_mbps{interface="eth0",server_ip="8.8.8.8"} 2003.0
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
