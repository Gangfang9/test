import "./App.scss";
import { Button, Flex, Layout, message, Spin, Typography } from "antd";
import { MessageContext, useDeviceWebSocket } from "./hooks";
import { staticStore, useAppDispatch, useAppSelector } from "./store/store";
import { forceSetLocalConfig } from "./store/localConfig";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { Content } from "antd/es/layout/layout";
import Sider from "./components/Sider";
import { useLocation, useOutlet } from "react-router-dom";
import KeepAlive, { useKeepAliveRef } from "keepalive-for-react";
import LoadingWrapper from "./components/common/LoadingWrapper";
import { requestGet } from "./utils";
import { setIsLoading } from "./store/other";
import i18n from "./i18n";
import Membership, { type MembershipStatus } from "./components/Membership";
import { requestPost } from "./utils";
import MembershipRecharge from "./components/MembershipRecharge";
import { formatMembershipExpiry, reportMembershipRendered } from "./membershipPresentation";

function App() {
  const [membership, setMembership] = useState<MembershipStatus | null>(null);
  const latest = useRef<MembershipStatus | null>(null);
  const epoch = useRef(0);
  const loggingOut = useRef(false);
  const acceptStatus = useCallback((status: MembershipStatus) => {
    if (loggingOut.current && status.logged_in) return;
    if ((status.revision ?? 0) < (latest.current?.revision ?? 0)) return;
    latest.current = status;
    document.documentElement.dataset.jxMember = String(status.member);
    setMembership(status);
  }, []);
  const onChange = useCallback((status: MembershipStatus) => {
    epoch.current += 1;
    acceptStatus(status);
  }, [acceptStatus]);

  const logout = useCallback(async () => {
    if (loggingOut.current) return;
    loggingOut.current = true;
    epoch.current += 1;
    const cleared = { ...latest.current, logged_in: false, member: false,
      account: undefined, membership_expires_at: null, lease_remaining_ms: 0 };
    // Unmount the protected page in this click, before contacting the backend.
    flushSync(() => acceptStatus(cleared));
    try {
      const result = await requestPost<MembershipStatus>("/api/member/logout", {});
      onChange(result.data);
      loggingOut.current = false;
    } catch {
      // Never restore an old session when the local logout request fails.
      message.error("本地退出未完成，请关闭软件后重新打开。");
    }
  }, [acceptStatus, onChange]);

  useEffect(() => {
    let current = true;
    let pending = false;
    const refresh = async () => {
      if (pending || loggingOut.current) return;
      pending = true;
      const requestEpoch = epoch.current;
      const started = performance.now();
      try {
        const result = await requestGet<MembershipStatus>("/api/member/status");
        if (current && requestEpoch === epoch.current) acceptStatus({ ...result.data,
          lease_remaining_ms: Math.max(0, (result.data.lease_remaining_ms ?? 0) - (performance.now() - started)),
        });
      } catch {
        if (current && requestEpoch === epoch.current) acceptStatus({ ...latest.current, member: false, logged_in: latest.current?.logged_in ?? false });
      } finally {
        pending = false;
      }
    };
    const nativeStatus = (event: Event) => {
      const status = (event as CustomEvent<MembershipStatus>).detail;
      if (!status || typeof status.member !== "boolean") return;
      epoch.current += 1;
      // Synchronous gate replacement before native window resizing.
      flushSync(() => acceptStatus(status));
    };
    window.addEventListener("jx-membership-status", nativeStatus);
    void refresh();
    const interval = window.setInterval(refresh, 1_000);
    return () => { current = false; window.clearInterval(interval); window.removeEventListener("jx-membership-status", nativeStatus); };
  }, [acceptStatus]);

  useLayoutEffect(() => {
    if (!membership) return;
    return reportMembershipRendered(membership);
  }, [membership?.member, membership?.revision]);

  useEffect(() => {
    if (!membership?.member) return;
    const status = membership;
    const requestEpoch = epoch.current;
    const timer = window.setTimeout(() => {
      if (requestEpoch === epoch.current) onChange({ ...status, member: false, lease_remaining_ms: 0 });
    }, Math.min(status.lease_remaining_ms ?? 0, 2_147_483_647));
    return () => window.clearTimeout(timer);
  }, [membership, onChange]);

  if (!membership) return <Spin spinning fullscreen tip="正在检查会员状态" />;
  if (!membership.member) return <Membership status={membership} onChange={onChange} onLogout={() => void logout()} />;
  return <AuthenticatedApp membership={membership} onChange={onChange} onLogout={() => void logout()} />;
}

function AuthenticatedApp({ membership, onChange, onLogout }: {
  membership: MembershipStatus;
  onChange: (status: MembershipStatus) => void;
  onLogout: () => void;
}) {
  const dispatch = useAppDispatch();
  const [messageApi, contextHolder] = message.useMessage();
  const isLoading = useAppSelector((state) => state.other.isLoading);
  const location = useLocation();
  const aliveRef = useKeepAliveRef();

  useDeviceWebSocket();

  const outlet = useOutlet();

  async function loadLocalConfig() {
    try {
      const res = await requestGet("/api/config/get_config");
      dispatch(forceSetLocalConfig(res.data));
      i18n.changeLanguage(res.data.language);
    } catch (err: any) {
      messageApi.error(err);
    }
  }

  useEffect(() => {
    staticStore.messageApi = messageApi;
    dispatch(setIsLoading(true));
    loadLocalConfig();
    dispatch(setIsLoading(false));

    // prevent backward
    history.pushState(null, "", window.location.href);
    const handlePopState = () => {
      history.pushState(null, "", window.location.href);
    };
    window.addEventListener("popstate", handlePopState);

    return () => {
      window.removeEventListener("popstate", handlePopState);
    };
  }, []);

  return (
    <MessageContext.Provider value={messageApi}>
      {contextHolder}
      <Spin spinning={isLoading} fullscreen delay={200} />
      <Layout className="min-h-100vh authenticated-app">
        <Sider />
        <Layout>
          <Flex className="membership-toolbar" align="center" justify="end" gap={12} style={{ padding: "6px 18px", borderBottom: "1px solid #e8edf5" }}>
            <Typography.Text type="secondary">会员：{membership.account}</Typography.Text>
            <Typography.Text type="secondary" title="北京时间">到期：{formatMembershipExpiry(membership.membership_expires_at)}</Typography.Text>
            <MembershipRecharge status={membership} onChange={onChange} onSuccess={() => { messageApi.success("卡密充值成功，到期时间已更新"); }} />
            <Button size="small" onClick={onLogout}>退出登录</Button>
          </Flex>
          <Content>
            <KeepAlive
              transition
              aliveRef={aliveRef}
              activeCacheKey={location.pathname}
            >
              <LoadingWrapper>
                <div className="page-container-parent scrollbar">{outlet}</div>
              </LoadingWrapper>
            </KeepAlive>
          </Content>
        </Layout>
      </Layout>
    </MessageContext.Provider>
  );
}

export default App;
