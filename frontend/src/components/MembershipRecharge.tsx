import { Alert, Button, Form, Input, Modal, Typography } from "antd";
import { useRef, useState } from "react";
import type { MembershipStatus } from "./Membership";
import { formatMembershipExpiry } from "../membershipPresentation";
import { requestPost } from "../utils";

export default function MembershipRecharge({ status, onChange, onSuccess }: {
  status: MembershipStatus;
  onChange: (status: MembershipStatus) => void;
  onSuccess: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const pending = useRef(false);
  const [form] = Form.useForm<{ card: string }>();

  async function submit({ card }: { card: string }) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setError("");
    try {
      const result = await requestPost<MembershipStatus>("/api/member/redeem", { card: card.trim() });
      onChange(result.data);
      if (result.data.member) {
        form.resetFields();
        setOpen(false);
        onSuccess();
      } else {
        setError("云端尚未确认会员有效，请检查会员状态后再试。");
      }
    } catch (reason) {
      setError(typeof reason === "string" ? reason : "充值失败，请稍后重试");
    } finally {
      pending.current = false;
      setBusy(false);
    }
  }

  return <>
    <Button size="small" type="primary" ghost onClick={() => { setError(""); setOpen(true); }}>卡密充值</Button>
    <Modal title="卡密充值" open={open} width={420} closable={!busy} maskClosable={!busy}
      onCancel={() => { if (!pending.current) setOpen(false); }}
      footer={[
        <Button key="cancel" disabled={busy} onClick={() => setOpen(false)}>取消</Button>,
        <Button key="confirm" type="primary" loading={busy} onClick={() => form.submit()}>确认充值</Button>,
      ]}>
      <Typography.Paragraph type="secondary">
        当前到期：{formatMembershipExpiry(status.membership_expires_at)}（北京时间）
      </Typography.Paragraph>
      {error && <Alert type="error" showIcon message={error} style={{ marginBottom: 16 }} />}
      <Form form={form} layout="vertical" onFinish={submit}>
        <Form.Item label="卡密" name="card" rules={[{ required: true, whitespace: true, message: "请输入卡密" }]}>
          <Input autoComplete="off" maxLength={128} disabled={busy} placeholder="请输入卡密" />
        </Form.Item>
      </Form>
      <Typography.Text type="secondary">充值成功后自动更新到期时间</Typography.Text>
    </Modal>
  </>;
}
