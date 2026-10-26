# TLS 证书验证回归夹具

这些文件只包含公开 X.509 证书，不包含私钥。测试离线运行，不访问原站点，
不读取或修改操作系统信任库。验证时间固定为 `2026-10-26T12:00:00Z`
（Unix 秒 `1793016000`），位于所有夹具的有效期内。

## 失信中间证书链

- `emsign-g3b-noct-chain.pem` 原样取自事件报告方的
  [`chains.zip`](https://emudhra.we-have-a-problem.com/chains.zip)，成员为
  `chains/dv-g3b-noct-emudhra.we-have-a-problem.com.pem`。
  顺序为测试站点叶证书、`EM DV TLS CA - G3B`、由 `emSign ECC Root CA - G3`
  交叉签名的 `emSign Root TLS CA - G3`。该叶证书不带 SCT。
- `emsign-ecc-root-g3.pem` 来自同站点的
  [`roots.json`](https://emudhra.we-have-a-problem.com/roots.json)，选择
  `name = emSign ECC Root CA - G3` 的 `pem` 字段并补充末尾换行。
  其 DER SHA-256 与源清单一致：
  `86a1ecba089c4a8d3bbe2734c612ba341d813e043cf9e8a862cd5c57a36bbe6b`。
- 来源说明：[事件报告](https://emudhra.we-have-a-problem.com/)、
  [RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382)、
  [WebPKI 修复](https://github.com/rustls/webpki/pull/546)。
- 获取日期：2026-10-26。源 ZIP SHA-256：
  `7b949956955fc501a30f4d98d2f513d34c0bbbe92e29a968194c96e097ce2345`；
  源 JSON SHA-256：
  `119f32cd81a6b5a4e3c4c2df5fe07daa90dc43cafa123a62f0394f1895d92380`。

负例显式信任原根证书，并要求 `rustls` 返回的底层错误为 `CertDistrusted`。
证书过期、缺少根证书、域名错误和签名错误均不能满足断言。这可防止后续锁文件
回退到仍接受该证书链的验证器；旧依赖是否接受同一链，以实际负对照运行结果为准。

## 正常证书链

三个 `rustls-p256-*.pem` 文件分别原样复制自 Rustls 官方 `v/0.23.39` 的
[`ca.cert`](https://github.com/rustls/rustls/blob/v/0.23.39/test-ca/ecdsa-p256/ca.cert)、
[`inter.cert`](https://github.com/rustls/rustls/blob/v/0.23.39/test-ca/ecdsa-p256/inter.cert)、
[`end.cert`](https://github.com/rustls/rustls/blob/v/0.23.39/test-ca/ecdsa-p256/end.cert)。
正例验证域名 `testserver.com`，防止把拒绝所有证书误判为安全修复。

## 保存文件摘要

| 文件 | SHA-256 |
| --- | --- |
| `emsign-ecc-root-g3.pem` | `36e68e205b53c67c7a013894e0d5c8583063468118d1ce78ecbc2200d1dd185c` |
| `emsign-g3b-noct-chain.pem` | `de597b4e346d6e84d264a6bc62f5cb5bf4a4d64ec5e587edc446016e4aa693c5` |
| `rustls-p256-root.pem` | `51edb8029a60e5bc1ee181860a9d2bbd4154d52c34a3ae40e4f01f332738340a` |
| `rustls-p256-intermediate.pem` | `019dd32ce72935dad054d3109ca8498aed1d1806f116a84fb433cfab33adf5c4` |
| `rustls-p256-leaf.pem` | `e3e70757d49633cc27bad48509c93c863b96f6a106a50b1483fd1318fc30172c` |

## 验证边界

测试使用工作区锁定的 `rustls::client::WebPkiServerVerifier`，显式选择现有
AWS-LC provider。结果仅说明该依赖组合的离线证书链验证行为，不代表真实 TLS
握手、系统信任库、macOS Security.framework 或 Windows 原生证书验证已经验收。
运行前必须遵守根 `AGENTS.md` 和本机 `.local/README.md` 的测试目录要求。

```sh
cargo nextest run --locked -p websocket --lib --no-fail-fast --no-tests fail -E 'test(bundled_tls)'
```
