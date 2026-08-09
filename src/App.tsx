import { BackupApp } from "./features/backup/BackupApp";
import { SettingsApp } from "./features/backup/SettingsApp";

export default function App() {
  const windowKind = new URLSearchParams(window.location.search).get("window");
  return windowKind === "settings" ? <SettingsApp /> : <BackupApp />;
}
