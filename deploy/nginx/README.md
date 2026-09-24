# admin.jxzs.top 后台入口（待备案通过后启用）

当前 U验证后台仍由 SSH 隧道访问：`http://127.0.0.1:19080/admin/`。本目录仅准备未来的网址入口，**尚未部署、未建立公网 DNS，也未签发证书**。`www.jxzs.top` 的会员 API 不在这次变更范围内。

## 启用条件

1. 确认 `jxzs.top` 对应的备案已通过，并确认新增管理子域名的接入要求。
2. 核实云服务器上的 U验证服务仍在 `127.0.0.1:19080` 可访问，且公网无法直连 19080；记录现有 Nginx 配置和回滚备份。
3. 确定管理员的固定公网出口 IP 或 VPN 出口 IP。若没有稳定可信的出口地址，先建立 VPN 或采用经验证的客户端证书方案，不要开放给所有地址。
4. 为 `admin.jxzs.top` 创建指向该服务器的 DNS A 记录，签发该域名的有效 HTTPS 证书。建议用 DNS-01；DNS API 凭据只能保存在服务器的私有凭据文件中，不能放进仓库或命令日志。

## 安装与启用

以下命令在云服务器执行，且应先按实际系统确认 Nginx 的 `sites-available` / `sites-enabled` 布局。不要在备案通过前执行 DNS 发布或启用步骤。

```bash
sudo install -m 0644 deploy/nginx/admin.jxzs.top.conf /etc/nginx/sites-available/admin.jxzs.top.conf
sudo install -m 0644 deploy/nginx/jx-admin-allowlist.conf /etc/nginx/snippets/jx-admin-allowlist.conf
```

先编辑 `/etc/nginx/snippets/jx-admin-allowlist.conf`，在 `deny all;` **之前**逐条加入可信源地址，例如 `allow 203.0.113.10/32;`。示例地址不能直接用于生产。保留最后的 `deny all;`。若 Nginx 前面还有负载均衡或 CDN，先核实真实客户端 IP 的可信代理配置，否则不要用 `X-Forwarded-For` 直接作为放行依据。

确认 `/etc/letsencrypt/live/admin.jxzs.top/` 下证书已签发且 Nginx 可读，再启用站点：

```bash
sudo ln -s /etc/nginx/sites-available/admin.jxzs.top.conf /etc/nginx/sites-enabled/admin.jxzs.top.conf
sudo nginx -t
sudo systemctl reload nginx
```

若 `nginx -t` 失败，不要 reload；修复证书路径、配置或与现有站点的冲突后重试。证书续期也需经过验证，避免后台入口因证书过期中断。

## 验收与回滚

- 从允许的出口 IP 打开 `https://admin.jxzs.top/admin/`，确认页面、静态资源与后台登录正常；从不在白名单中的出口 IP 访问应返回 403。检查证书域名及有效期。
- 确认公网对 19080 端口仍不可达，会员 API 和原 SSH 隧道仍正常。网址入口验收完成前保留 SSH 隧道作为回退方式。
- 如需回滚，在服务器上删除本次新建的 `sites-enabled/admin.jxzs.top.conf` 链接，运行 `sudo nginx -t && sudo systemctl reload nginx`；保留证书和配置供排查。回滚不会修改 U验证服务或 `www.jxzs.top`。

后台登录自身的密码和二次验证能力必须在 U验证系统中单独核实；IP 白名单不能替代管理员认证。历史记录中的服务器密码已暴露，不应继续使用，应先轮换并改用 SSH 密钥。
