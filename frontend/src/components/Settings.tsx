import { Alert, Button, Card, Flex, Select, Slider, Switch, Typography } from "antd";
import { ReloadOutlined } from "@ant-design/icons";
import type { ReactNode } from "react";
import { useAppDispatch, useAppSelector } from "../store/store";
import {
  forceSetLocalConfig,
  setAlwaysOnTop,
  setAudioBitRate,
  setAudioCodec,
  setAudioEnabled,
  setClipboardSync,
  setMappingLabelOpacity,
  setStayAwake,
  setTitlebarVisible,
  setVideoBitRate,
  setVideoCodec,
  setVideoMaxFps,
  setVideoMaxSize,
} from "../store/localConfig";
import { setIsLoading } from "../store/other";
import { useMessageContext } from "../hooks";
import { requestGet } from "../utils";

type SettingRowProps = {
  label: string;
  description?: string;
  children: ReactNode;
};

function SettingRow({ label, description, children }: SettingRowProps) {
  return (
    <Card size="small" className="product-setting-row">
      <Flex align="center" justify="space-between" gap="large" wrap="wrap">
        <div>
          <Typography.Text strong>{label}</Typography.Text>
          {description && (
            <div>
              <Typography.Text type="secondary" className="text-3">
                {description}
              </Typography.Text>
            </div>
          )}
        </div>
        <div>{children}</div>
      </Flex>
    </Card>
  );
}

const maxSizeOptions = [
  { value: 0, label: "原始分辨率" },
  { value: 360, label: "360p" },
  { value: 480, label: "480p" },
  { value: 720, label: "720p" },
  { value: 1080, label: "1080p" },
];

const fpsOptions = [
  { value: 0, label: "不限制" },
  { value: 30, label: "30 FPS" },
  { value: 60, label: "60 FPS" },
  { value: 90, label: "90 FPS" },
  { value: 120, label: "120 FPS" },
];

const bitRateOptions = [4, 8, 10, 16, 20].map((value) => ({
  value: value * 1_000_000,
  label: `${value} Mbps`,
}));

export default function Settings() {
  const dispatch = useAppDispatch();
  const config = useAppSelector((state) => state.localConfig);
  const messageApi = useMessageContext();

  async function reloadConfig() {
    dispatch(setIsLoading(true));
    try {
      const response = await requestGet("/api/config/get_config");
      dispatch(forceSetLocalConfig(response.data));
      messageApi?.success("设置已从程序重新读取");
    } catch (error) {
      messageApi?.error(error as string);
    } finally {
      dispatch(setIsLoading(false));
    }
  }

  return (
    <div className="page-container product-settings">
      <Flex align="center" justify="space-between" className="mb-5">
        <h2 className="title-with-line" style={{ marginBottom: 0 }}>设置</h2>
        <Button icon={<ReloadOutlined />} onClick={reloadConfig}>重新读取</Button>
      </Flex>

      <Alert showIcon type="info" className="mb-5" message="画质、帧率、编码和声音设置会在下次投屏或点击“重连”后生效。" />

      <h3 className="title-with-line-sub">投屏</h3>
      <Flex vertical gap="small">
        <SettingRow label="分辨率" description="限制最长边；原始分辨率画质最高">
          <Select className="w-9rem" value={config.videoMaxSize} options={maxSizeOptions} onChange={(value) => dispatch(setVideoMaxSize(value))} />
        </SettingRow>
        <SettingRow label="FPS" description="手机和编码器不支持时会自动降低">
          <Select className="w-9rem" value={config.videoMaxFps} options={fpsOptions} onChange={(value) => dispatch(setVideoMaxFps(value))} />
        </SettingRow>
        <SettingRow label="视频码率" description="码率越高画面越清晰，USB 带宽占用也越高">
          <Select className="w-9rem" value={config.videoBitRate} options={bitRateOptions} onChange={(value) => dispatch(setVideoBitRate(value))} />
        </SettingRow>
        <SettingRow label="视频编码" description="H.264 兼容性最好；H.265/AV1 取决于手机支持">
          <Select className="w-9rem" value={config.videoCodec} options={["H264", "H265", "AV1"].map((value) => ({ value, label: value }))} onChange={(value) => dispatch(setVideoCodec(value))} />
        </SettingRow>
        <SettingRow label="音频转发（Android 11+）" description="打开后声音从手机传到电脑">
          <Switch checked={config.audioEnabled} onChange={(value) => dispatch(setAudioEnabled(value))} />
        </SettingRow>
        {config.audioEnabled && (
          <>
            <SettingRow label="音频编码">
              <Select className="w-9rem" value={config.audioCodec} options={["OPUS", "AAC", "FLAC", "RAW"].map((value) => ({ value, label: value }))} onChange={(value) => dispatch(setAudioCodec(value))} />
            </SettingRow>
            <SettingRow label="音频码率">
              <Select className="w-9rem" value={config.audioBitRate} options={[64_000, 128_000, 256_000].map((value) => ({ value, label: `${value / 1000} Kbps` }))} onChange={(value) => dispatch(setAudioBitRate(value))} />
            </SettingRow>
          </>
        )}
      </Flex>

      <h3 className="title-with-line-sub">显示与控制</h3>
      <Flex vertical gap="small">
        <SettingRow label="窗口置顶" description="让投屏窗口保持在其他窗口前面">
          <Switch checked={config.alwaysOnTop} onChange={(value) => dispatch(setAlwaysOnTop(value))} />
        </SettingRow>
        <SettingRow label="显示投屏标题栏" description="显示返回、主页、最近任务和电源等真实控制按钮">
          <Switch checked={config.titlebarVisible} onChange={(value) => dispatch(setTitlebarVisible(value))} />
        </SettingRow>
        <SettingRow label="按键映射透明度" description="调整投屏窗口上的按键提示透明度">
          <Slider className="w-14rem" min={0} max={1} step={0.05} value={config.mappingLabelOpacity} tooltip={{ formatter: (value) => `${Math.round((value ?? 0) * 100)}%` }} onChange={(value) => dispatch(setMappingLabelOpacity(value))} />
        </SettingRow>
        <SettingRow label="保持唤醒" description="投屏期间通过 scrcpy 服务阻止设备自动休眠">
          <Switch checked={config.stayAwake} onChange={(value) => dispatch(setStayAwake(value))} />
        </SettingRow>
        <SettingRow label="剪贴板同步" description="允许电脑与手机同步复制的文本">
          <Switch checked={config.clipboardSync} onChange={(value) => dispatch(setClipboardSync(value))} />
        </SettingRow>
      </Flex>

      <Alert className="mt-5" type="warning" showIcon message="区域投屏、录屏和暂停尚未接通完整的数据链路，因此本版不显示这些入口。" />
    </div>
  );
}
