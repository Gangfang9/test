import type { MembershipStatus } from "./components/Membership";

export function formatMembershipExpiry(expiry?: number | null): string {
  if (expiry == null) return "正在获取";
  if (expiry <= 0) return "未开通";
  const parts = new Intl.DateTimeFormat("zh-CN", {
    timeZone: "Asia/Shanghai", year: "numeric", month: "2-digit", day: "2-digit",
    hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23",
  }).formatToParts(new Date(expiry * 1000));
  const value = (type: Intl.DateTimeFormatPartTypes) => parts.find((part) => part.type === type)?.value;
  return `${value("year")}-${value("month")}-${value("day")} ${value("hour")}:${value("minute")}:${value("second")}`;
}

export function reportMembershipRendered(status: MembershipStatus): () => void {
  let secondFrame = 0;
  const firstFrame = requestAnimationFrame(() => {
    secondFrame = requestAnimationFrame(() => {
      const ipc = (window as Window & { ipc?: { postMessage: (body: string) => void } }).ipc;
      ipc?.postMessage(JSON.stringify({
        kind: "membership-rendered", revision: status.revision ?? 0, member: status.member,
      }));
    });
  });
  return () => { cancelAnimationFrame(firstFrame); cancelAnimationFrame(secondFrame); };
}
