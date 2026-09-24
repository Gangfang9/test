import { Alert, Button, Card, Form, Input, Space, Tabs, Typography, message } from "antd";
import { useState } from "react";
import { requestPost } from "../utils";

export type MembershipStatus = {
  logged_in: boolean;
  member: boolean;
  account?: string;
  device_suffix?: string;
};

type Credentials = { account: string; password: string };

export default function Membership({ status, onChange }: {
  status: MembershipStatus;
  onChange: (status: MembershipStatus) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [activeTab, setActiveTab] = useState("login");
  const [form] = Form.useForm<Credentials>();
  const [cardForm] = Form.useForm<{ card: string }>();

  async function submit(path: string, values: Record<string, string>) {
    setBusy(true);
    setError("");
    setNotice("");
    let registrationCreated = false;
    try {
      if (path === "register") {
        const credentials = { account: values.account, password: values.password };
        await requestPost<unknown>("/api/member/register", credentials);
        registrationCreated = true;
        form.setFieldValue("account", credentials.account);

        const card = values.card?.trim();
        if (!card) {
          setNotice("注册成功，请登录后使用卡密开通会员。");
          setActiveTab("login");
          return;
        }

        setNotice("账号已创建，正在登录并充值卡密。");
        const login = await requestPost<MembershipStatus>("/api/member/login", credentials);
        onChange(login.data);
        const redemption = await requestPost<MembershipStatus>("/api/member/redeem", { card });
        onChange(redemption.data);
        if (redemption.data.member) {
          message.success("注册并充值成功");
        } else {
          setNotice("账号已创建，但卡密未开通会员；可在当前页面重新充值。");
        }
        return;
      }

      const result = await requestPost<MembershipStatus>(`/api/member/${path}`, values);
      onChange(result.data);
      if (path === "login" && !result.data.member) {
        setNotice("登录成功，当前账号尚未开通会员。请充值卡密。");
      }
      if (path === "redeem") {
        cardForm.resetFields();
        message.success("卡密充值成功");
      }
    } catch (reason) {
      if (registrationCreated) {
        setNotice("账号已创建；自动登录或充值未完成。请登录后继续充值。");
        setActiveTab("login");
      }
      setError(typeof reason === "string" ? reason : "操作失败，请稍后重试");
    } finally {
      setBusy(false);
    }
  }

  return <div className="membership-page">
    <Card className="membership-card" bordered>
      <Typography.Title level={2} style={{ marginBottom: 4 }}>JX手游助手</Typography.Title>
      {status.device_suffix && <Typography.Paragraph type="secondary" style={{ marginTop: 10 }}>
        本机设备码末八位：{status.device_suffix}
      </Typography.Paragraph>}
      {notice && <Alert type="success" showIcon message={notice} style={{ marginTop: 16 }} />}
      {error && <Alert type="error" showIcon message={error} style={{ marginTop: 16 }} />}
      {status.logged_in ? <div style={{ marginTop: 24 }}>
        <Typography.Paragraph>账号：{status.account}</Typography.Paragraph>
        <Alert type="warning" showIcon message="会员未开通或已到期" description="充值有效卡密后才能使用投屏和键鼠映射。" />
        <Form form={cardForm} layout="vertical" onFinish={(values) => submit("redeem", values)} style={{ marginTop: 20 }}>
          <Form.Item label="卡密" name="card" rules={[{ required: true, message: "请输入卡密" }]}>
            <Input autoComplete="off" maxLength={128} placeholder="输入天卡、周卡或月卡卡密" />
          </Form.Item>
          <Space>
            <Button type="primary" htmlType="submit" loading={busy}>充值并验证</Button>
            <Button disabled={busy} onClick={async () => {
              try { const result = await requestPost<MembershipStatus>("/api/member/logout", {}); onChange(result.data); }
              catch { onChange({ logged_in: false, member: false }); }
            }}>退出登录</Button>
          </Space>
        </Form>
      </div> : <Tabs activeKey={activeTab} onChange={setActiveTab} style={{ marginTop: 20 }} items={[
        { key: "login", label: "登录", children: <Form form={form} layout="vertical" onFinish={(values) => submit("login", values)}>
          <Form.Item label="账号" name="account" rules={[{ required: true, min: 6, max: 18 }]}>
            <Input autoComplete="username" maxLength={18} />
          </Form.Item>
          <Form.Item label="密码" name="password" rules={[{ required: true, min: 6, max: 18 }]}>
            <Input.Password autoComplete="current-password" maxLength={18} />
          </Form.Item>
          <Button type="primary" htmlType="submit" loading={busy} block>登录并验证</Button>
        </Form> },
        { key: "register", label: "注册", children: <Form layout="vertical" onFinish={(values) => submit("register", values)}>
          <Form.Item label="账号" name="account" rules={[{ required: true, min: 6, max: 18, message: "账号长度需为 6–18 位" }]}>
            <Input autoComplete="username" maxLength={18} placeholder="账号长度 6–18 位" />
          </Form.Item>
          <Form.Item label="密码" name="password" rules={[{ required: true, min: 6, max: 18, message: "密码长度需为 6–18 位" }]}>
            <Input.Password autoComplete="new-password" maxLength={18} placeholder="密码长度 6–18 位" />
          </Form.Item>
          <Form.Item label="卡密（可选）" name="card">
            <Input autoComplete="off" maxLength={128} placeholder="填写后注册成功将自动充值" />
          </Form.Item>
          <Typography.Paragraph type="secondary">每台电脑 24 小时内只能成功注册一个账号。可在注册时填写卡密，也可登录后充值。</Typography.Paragraph>
          <Button type="primary" htmlType="submit" loading={busy} block>创建账号</Button>
        </Form> },
      ]} />}
    </Card>
  </div>;
}
