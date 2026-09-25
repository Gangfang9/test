# jxzs.host.mg 会员 API 入口

状态：2026-09-25 已在服务器启用 HTTPS 会员接口。`www.jxzs.top` 原站点保持不变。客户端的新主地址和旧地址回退逻辑已在源码中修改，尚待构建与实际账号验收。

现场进度：Nginx、会员网关、U验证运行；本机网关空登录请求返回 400。域名 DNS 曾短暂委派到未配置的 DNSPod 权威服务器，导致 Certbot 首次申请时遇到 NXDOMAIN；用户更新后，两台 Cloudflare 权威服务器及 Google、Cloudflare 递归解析器均返回 `62.234.62.154`。第二次申请成功，证书到期日为 2026-12-24，续期演练通过。外部 HTTPS 空登录请求返回网关预期的 400，`/admin/` 与 `/health` 返回 404。

## 仍需确认

1. 用户表示域名已备案；未独立取得 `host.mg` 所属二级域名的备案及腾讯云接入凭证、子域名使用授权。`jxzs.top` 的备案不覆盖 `host.mg`。
2. 用测试账号完成注册、登录、充值、心跳、登出及跨设备验证；不要把空参数返回 400 当作完整会员验收。
3. 验证新客户端的 Windows 构建和主地址故障时的旧地址回退。当前仅在连接建立前失败时回退，不重试可能已被处理的超时或业务错误。
4. 后续轮换已在历史对话暴露的 SSH 密码，并启用独立 SSH 密钥；继续保留 UFW 默认拒绝入站和仅允许 22/80/443 的规则。现场检查：19081 仅监听 127.0.0.1，19080 虽监听所有网卡，但 UFW 未允许公网访问，外部 HTTP 探测超时。

## 已执行的部署步骤（不要重复创建站点链接）

已将两个配置文件复制到服务器的 `/tmp/`。下面记录当时执行的步骤；服务器上已有 `/etc/nginx/sites-enabled/jxzs.host.mg.conf` 链接，勿重复执行 `ln -s`。

```bash
sudo mkdir -p /var/www/jxzs-host-mg-acme/.well-known/acme-challenge
sudo install -m 0644 /tmp/jxzs.host.mg.http.conf /etc/nginx/sites-available/jxzs.host.mg.conf
sudo ln -s /etc/nginx/sites-available/jxzs.host.mg.conf /etc/nginx/sites-enabled/jxzs.host.mg.conf
sudo nginx -t && sudo systemctl reload nginx
```

先在 `/var/www/jxzs-host-mg-acme/.well-known/acme-challenge/` 建立临时测试文件，外部 HTTP 读取成功后已删除。Certbot 通过 HTTP-01 签发证书。

```bash
sudo certbot certonly --webroot -w /var/www/jxzs-host-mg-acme -d jxzs.host.mg
sudo certbot renew --dry-run
```

证书就绪后，安装最终站点配置并检查：

```bash
sudo install -m 0644 /tmp/jxzs.host.mg.conf /etc/nginx/sites-available/jxzs.host.mg.conf
sudo nginx -t && sudo systemctl reload nginx
```

已从外部网络以有效证书向 `https://jxzs.host.mg/v1/login` POST 空 JSON，收到网关业务参数错误。`/admin/`、`/health` 返回 404。服务器已安装 `/etc/letsencrypt/renewal-hooks/deploy/jx-reload-nginx.sh`，证书续期后会在 Nginx 语法检查通过时自动重载；脚本已单独执行验证。

客户端源码已将 `https://jxzs.host.mg` 设为主地址，`https://www.jxzs.top` 设为连接失败时的备选地址；尚未发布新 Windows 包。服务器旧入口的 Nginx 站点保持不变。
