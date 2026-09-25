# jxzs.host.mg 会员 API 备选入口（准备稿）

状态：仅本地配置，**尚未启用服务器、签发证书或修改客户端**。`www.jxzs.top` 原入口保持不变。用户已将 `jxzs.host.mg` 的 A 记录指向服务器，但 DNS 生效不代表 HTTPS 或会员网关可用。

## 上线前必须确认

1. 核实 `host.mg` 所属二级域名的 ICP 备案、腾讯云接入状态及子域名使用授权。`jxzs.top` 的备案不覆盖 `host.mg`；当前用户尚不确定其状态。确认前不要在中国内地服务器上启用公网会员站点。
2. 获得可用的服务器 SSH 管理方式，先备份并检查现有 `nginx -T` 配置。当前自动化 SSH 密钥登录被拒绝；不要使用已在历史对话暴露的旧密码。
3. 在服务器确认 `jx-membership` 仍只监听 `127.0.0.1:19081`，本机向 `/v1/login` POST 空 JSON 能得到业务参数错误；确保 U验证 19080 与网关 19081 未开放公网。

## 核实条件后安装

先将本目录的两个配置文件复制到服务器的 `/tmp/`，再执行以下示例命令。命令按 Ubuntu 默认 `sites-available` 布局编写；执行前需核对现有 Nginx 安装和站点名称，避免覆盖或重复 `server_name`。

```bash
sudo mkdir -p /var/www/jxzs-host-mg-acme/.well-known/acme-challenge
sudo install -m 0644 /tmp/jxzs.host.mg.http.conf /etc/nginx/sites-available/jxzs.host.mg.conf
sudo ln -s /etc/nginx/sites-available/jxzs.host.mg.conf /etc/nginx/sites-enabled/jxzs.host.mg.conf
sudo nginx -t && sudo systemctl reload nginx
```

先在 `/var/www/jxzs-host-mg-acme/.well-known/acme-challenge/` 建立临时测试文件，从外部网络确认该文件能通过 HTTP 读取，随后删除。再用已安装的 Certbot 通过 HTTP-01 签发证书。Certbot 的 webroot 路径须和上面的 `root` 一致；若域名服务商允许 TXT 记录，也可用支持自动续期的 DNS-01 插件。

```bash
sudo certbot certonly --webroot -w /var/www/jxzs-host-mg-acme -d jxzs.host.mg
sudo certbot renew --dry-run
```

证书就绪后，安装最终站点配置并检查：

```bash
sudo install -m 0644 /tmp/jxzs.host.mg.conf /etc/nginx/sites-available/jxzs.host.mg.conf
sudo nginx -t && sudo systemctl reload nginx
```

从外部网络以有效证书向 `https://jxzs.host.mg/v1/login` POST 空 JSON，应收到网关的业务参数错误，而不是 404、重定向到拦截页或 TLS 错误。验证注册、登录、充值、心跳与退出；确认 `/admin/`、`/health` 和其他路径仍为 404。不要用真实密码或卡密做冒烟测试。

只有新入口完成端到端验证后，才在客户端增加备选地址并重新构建。客户端目前固定使用 `https://www.jxzs.top`，尚未向 `jxzs.host.mg` 发送任何会员凭据。旧入口的 Nginx 站点和客户端配置在切换前保持不变。
