import "./App.scss";
import { Button, Flex, Layout, message, Spin, Typography } from "antd";
import { MessageContext, useDeviceWebSocket } from "./hooks";
import { staticStore, useAppDispatch, useAppSelector } from "./store/store";
import { forceSetLocalConfig } from "./store/localConfig";
import { useEffect, useState } from "react";
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

function App() {
  const [membership, setMembership] = useState<MembershipStatus | null>(null);
  useEffect(() => {
    let current = true;
    const refresh = async () => {
      try {
        const result = await requestGet<MembershipStatus>("/api/member/status");
        if (current) setMembership(result.data);
      } catch {
        if (current) setMembership({ logged_in: false, member: false });
      }
    };
    void refresh();
    const interval = window.setInterval(refresh, 10_000);
    return () => { current = false; window.clearInterval(interval); };
  }, []);

  if (!membership) return <Spin spinning fullscreen tip="正在检查会员状态" />;
  if (!membership.member) return <Membership status={membership} onChange={setMembership} />;
  return <AuthenticatedApp membership={membership} onChange={setMembership} />;
}

function AuthenticatedApp({ membership, onChange }: {
  membership: MembershipStatus;
  onChange: (status: MembershipStatus) => void;
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
      <Layout className="min-h-100vh">
        <Sider />
        <Layout>
          <Flex align="center" justify="end" gap={12} style={{ padding: "6px 18px", borderBottom: "1px solid #e8edf5" }}>
            <Typography.Text type="secondary">会员：{membership.account}</Typography.Text>
            <Button size="small" onClick={async () => {
              try {
                const result = await requestPost<MembershipStatus>("/api/member/logout", {});
                onChange(result.data);
              } catch {
                onChange({ logged_in: false, member: false });
              }
            }}>退出登录</Button>
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
