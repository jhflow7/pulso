import { FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  AlertCircle,
  ArrowDownToLine,
  ArrowUpFromLine,
  Check,
  CheckCircle2,
  ChevronRight,
  CircleHelp,
  ClipboardPaste,
  Clock3,
  Copy,
  Download,
  ExternalLink,
  FileArchive,
  FileAudio,
  FileImage,
  FileText,
  FolderOpen,
  Gauge,
  HardDrive,
  Link2,
  LoaderCircle,
  Pause,
  Play,
  Plus,
  RefreshCw,
  RotateCcw,
  Search,
  Settings2,
  ShieldCheck,
  Sparkles,
  Trash2,
  Wifi,
  X,
  Zap,
  CheckCheck,
  SlidersHorizontal,
} from "lucide-react";

type DownloadStatus = "active" | "waiting" | "paused" | "complete" | "error" | "removed" | string;
type DownloadItem = {
  gid: string;
  name: string;
  status: DownloadStatus;
  totalLength: number;
  completedLength: number;
  downloadSpeed: number;
  uploadSpeed: number;
  connections: number;
  errorMessage: string;
  directory: string;
  filePath: string;
  sourceUrl: string;
  tuningProfile: string;
};
type Stats = {
  downloadSpeed: number;
  uploadSpeed: number;
  active: number;
  waiting: number;
  stopped: number;
};
type EngineInfo = {
  downloadDir: string;
  aria2Version: string;
  appVersion: string;
};
type Preferences = {
  downloadDir: string;
  profile: "eco" | "balanced" | "turbo" | "custom" | string;
  autoProfile: boolean;
  maxConcurrentDownloads: number;
  connectionsPerServer: number;
  split: number;
  minSplitSize: string;
  maxDownloadSpeed: string;
  maxUploadSpeed: string;
  speedUnit: "MB" | "Mb" | string;
  notifyOnComplete: boolean;
};
type Filter = "all" | "active" | "waiting" | "paused" | "complete" | "error";
type ToastState = { message: string; kind: "success" | "error" | "info" };

const ZERO_STATS: Stats = { downloadSpeed: 0, uploadSpeed: 0, active: 0, waiting: 0, stopped: 0 };
const PROFILES: Record<string, Pick<Preferences, "maxConcurrentDownloads" | "connectionsPerServer" | "split" | "minSplitSize">> = {
  eco: { maxConcurrentDownloads: 3, connectionsPerServer: 4, split: 4, minSplitSize: "2M" },
  balanced: { maxConcurrentDownloads: 5, connectionsPerServer: 8, split: 8, minSplitSize: "1M" },
  turbo: { maxConcurrentDownloads: 8, connectionsPerServer: 16, split: 16, minSplitSize: "1M" },
};

function formatBytes(bytes: number, precision = 1): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const amount = bytes / Math.pow(1024, index);
  return `${amount.toLocaleString("pt-BR", { maximumFractionDigits: amount >= 100 ? 0 : precision })} ${units[index]}`;
}

function formatSpeed(bytesPerSecond: number, unit: string): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) return `0 ${unit}/s`;
  if (unit === "Mb") {
    const bits = bytesPerSecond * 8;
    const units = ["b", "Kb", "Mb", "Gb"];
    const index = Math.min(Math.floor(Math.log(bits) / Math.log(1000)), units.length - 1);
    return `${(bits / Math.pow(1000, index)).toLocaleString("pt-BR", { maximumFractionDigits: 1 })} ${units[index]}/s`;
  }
  return `${formatBytes(bytesPerSecond)}/s`;
}

function formatPercent(done: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(100, Math.max(0, (done / total) * 100));
}

function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0 || seconds > 365 * 24 * 3600) return "—";
  const whole = Math.ceil(seconds);
  const days = Math.floor(whole / 86400);
  const hours = Math.floor((whole % 86400) / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const secs = whole % 60;
  if (days) return `${days}d ${hours}h`;
  if (hours) return `${hours}h ${minutes}m`;
  if (minutes) return `${minutes}m ${secs}s`;
  return `${secs}s`;
}

function statusText(status: string): string {
  switch (status) {
    case "active": return "Baixando";
    case "waiting": return "Na fila";
    case "paused": return "Pausado";
    case "complete": return "Concluído";
    case "error": return "Falhou";
    case "removed": return "Removido";
    default: return status || "Preparando";
  }
}

function fileIcon(name: string) {
  const extension = name.split(".").pop()?.toLowerCase() ?? "";
  if (["zip", "7z", "rar", "tar", "gz", "xz", "iso", "bz2", "zst"].includes(extension)) return <FileArchive size={18} />;
  if (["jpg", "jpeg", "png", "webp", "gif", "svg", "bmp", "mp4", "mkv", "webm", "mov"].includes(extension)) return <FileImage size={18} />;
  if (["mp3", "flac", "wav", "ogg", "opus", "m4a", "aac"].includes(extension)) return <FileAudio size={18} />;
  if (["pdf", "txt", "md", "doc", "docx", "epub", "odt", "csv", "json"].includes(extension)) return <FileText size={18} />;
  return <Download size={18} />;
}

function sourceLabel(raw: string): string {
  if (!raw) return "Origem não informada";
  if (raw.startsWith("torrent-file:")) return "Arquivo .torrent";
  if (raw.toLowerCase().startsWith("magnet:")) return "Rede BitTorrent";
  try { return new URL(raw).hostname; } catch { return raw.length > 36 ? `${raw.slice(0, 33)}…` : raw; }
}

function SpeedSparkline({ values }: { values: number[] }) {
  const max = Math.max(1, ...values);
  const points = values.map((value, index) => {
    const x = values.length <= 1 ? 0 : (index / (values.length - 1)) * 320;
    const y = 43 - (value / max) * 34;
    return `${x},${y}`;
  }).join(" ");
  return (
    <svg className="speed-chart" viewBox="0 0 320 48" preserveAspectRatio="none" role="img" aria-label="Histórico recente da velocidade de download">
      <defs>
        <linearGradient id="pulso-chart-fill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#a996ff" stopOpacity="0.3" />
          <stop offset="100%" stopColor="#a996ff" stopOpacity="0" />
        </linearGradient>
      </defs>
      <polygon points={`0,48 ${points} 320,48`} fill="url(#pulso-chart-fill)" />
      <polyline points={points} fill="none" stroke="#a996ff" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

function App() {
  const [engine, setEngine] = useState<EngineInfo | null>(null);
  const [startError, setStartError] = useState("");
  const [connected, setConnected] = useState(false);
  const [lastSync, setLastSync] = useState<Date | null>(null);
  const [downloads, setDownloads] = useState<DownloadItem[]>([]);
  const [stats, setStats] = useState<Stats>(ZERO_STATS);
  const [speedHistory, setSpeedHistory] = useState<number[]>(Array.from({ length: 32 }, () => 0));
  const [smoothedRates, setSmoothedRates] = useState<Record<string, number>>({});
  const [smoothedGlobalRate, setSmoothedGlobalRate] = useState(0);
  const rateEma = useRef<Map<string, number>>(new Map());
  const globalRateEma = useRef(0);
  const [preferences, setPreferences] = useState<Preferences | null>(null);
  const [draftPreferences, setDraftPreferences] = useState<Preferences | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [linkText, setLinkText] = useState("");
  const [fileName, setFileName] = useState("");
  const [targetDir, setTargetDir] = useState("");
  const [search, setSearch] = useState("");
  const [adding, setAdding] = useState(false);
  const [busyGid, setBusyGid] = useState<string | null>(null);
  const [toast, setToast] = useState<ToastState | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [advancedAddOpen, setAdvancedAddOpen] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [savingSettings, setSavingSettings] = useState(false);
  const polling = useRef(false);

  const announce = useCallback((message: string, kind: ToastState["kind"] = "success") => {
    setToast({ message, kind });
  }, []);

  const refreshData = useCallback(async () => {
    if (polling.current) return;
    polling.current = true;
    try {
      const [nextDownloads, nextStats] = await Promise.all([
        invoke<DownloadItem[]>("get_downloads"),
        invoke<Stats>("get_stats"),
      ]);
      setDownloads(nextDownloads);
      setStats(nextStats);
      const rates: Record<string, number> = {};
      const tracked = new Set<string>();
      for (const item of nextDownloads) {
        tracked.add(item.gid);
        const previous = rateEma.current.get(item.gid) ?? 0;
        let nextRate = 0;
        if (item.status === "active") {
          nextRate = item.downloadSpeed > 0
            ? (previous > 0 ? previous * 0.68 + item.downloadSpeed * 0.32 : item.downloadSpeed)
            : previous * 0.82;
        }
        if (nextRate > 1) {
          rateEma.current.set(item.gid, nextRate);
          rates[item.gid] = nextRate;
        } else {
          rateEma.current.delete(item.gid);
        }
      }
      for (const gid of rateEma.current.keys()) {
        if (!tracked.has(gid)) rateEma.current.delete(gid);
      }
      setSmoothedRates(rates);
      const previousGlobal = globalRateEma.current;
      const nextGlobal = nextStats.active === 0
        ? 0
        : nextStats.downloadSpeed > 0
          ? (previousGlobal > 0 ? previousGlobal * 0.72 + nextStats.downloadSpeed * 0.28 : nextStats.downloadSpeed)
          : previousGlobal * 0.86;
      globalRateEma.current = nextGlobal > 1 ? nextGlobal : 0;
      setSmoothedGlobalRate(globalRateEma.current);
      setConnected(true);
      setLastSync(new Date());
      setSpeedHistory((current) => [...current.slice(-39), nextStats.downloadSpeed]);
    } catch (error) {
      console.warn("Pulso: a atualização falhou", error);
      setConnected(false);
    } finally {
      polling.current = false;
    }
  }, []);

  useEffect(() => {
    let alive = true;
    let timer: number | undefined;
    const boot = async () => {
      try {
        const info = await invoke<EngineInfo>("initialize");
        const prefs = await invoke<Preferences>("get_preferences");
        if (!alive) return;
        setEngine(info);
        setPreferences(prefs);
        setTargetDir("");
        await refreshData();
        if (alive) timer = window.setInterval(() => void refreshData(), 1000);
      } catch (error) {
        if (alive) setStartError(String(error));
      }
    };
    void boot();
    return () => {
      alive = false;
      if (timer !== undefined) window.clearInterval(timer);
    };
  }, [refreshData]);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), toast.kind === "error" ? 6000 : 3400);
    return () => window.clearTimeout(timer);
  }, [toast]);

  const filteredDownloads = useMemo(() => {
    const normalized = search.trim().toLowerCase();
    return downloads.filter((item) => {
      const matchesFilter = filter === "all"
        || (filter === "active" && item.status === "active")
        || (filter === "waiting" && item.status === "waiting")
        || (filter === "paused" && item.status === "paused")
        || (filter === "complete" && item.status === "complete")
        || (filter === "error" && item.status === "error");
      const matchesSearch = !normalized
        || item.name.toLowerCase().includes(normalized)
        || item.gid.toLowerCase().includes(normalized)
        || sourceLabel(item.sourceUrl).toLowerCase().includes(normalized);
      return matchesFilter && matchesSearch;
    });
  }, [downloads, filter, search]);

  const activeCount = downloads.filter((item) => item.status === "active").length;
  const waitingCount = downloads.filter((item) => item.status === "waiting").length;
  const completedCount = downloads.filter((item) => item.status === "complete").length;
  const failedCount = downloads.filter((item) => item.status === "error").length;
  const pausedCount = downloads.filter((item) => item.status === "paused").length;
  const liveCount = activeCount + waitingCount + pausedCount;
  // A fila geral exclui itens pausados, pois eles não têm previsão de conclusão até serem retomados.
  const totalRemaining = downloads.filter((item) => ["active", "waiting"].includes(item.status) && item.totalLength > 0)
    .reduce((sum, item) => sum + Math.max(0, item.totalLength - item.completedLength), 0);
  const effectiveGlobalRate = smoothedGlobalRate > 1 ? smoothedGlobalRate : stats.downloadSpeed;
  const etaSeconds = totalRemaining > 0 && effectiveGlobalRate > 1 ? totalRemaining / effectiveGlobalRate : null;
  const parsedLinks = useMemo(() => {
    const seen = new Set<string>();
    return linkText.split(/\r?\n/).map((line) => line.trim()).filter((line) => line && !line.startsWith("#"))
      .filter((line) => { if (seen.has(line)) return false; seen.add(line); return true; });
  }, [linkText]);

  const submitDownload = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!parsedLinks.length || adding) return;
    if (parsedLinks.length > 1 && fileName.trim()) {
      announce("Para vários links, deixe o campo de nome vazio.", "info");
      setAdvancedAddOpen(true);
      return;
    }
    setAdding(true);
    try {
      const gids = await invoke<string[]>("add_downloads", {
        urls: parsedLinks,
        fileName: fileName.trim() || null,
        targetDir: targetDir.trim() || null,
      });
      setLinkText("");
      setFileName("");
      setTargetDir("");
      setFilter("all");
      setAdvancedAddOpen(false);
      announce(gids.length === 1 ? "Download adicionado à fila." : `${gids.length} downloads adicionados à fila.`);
      await refreshData();
    } catch (error) {
      announce(`Não foi possível adicionar: ${String(error)}`, "error");
    } finally {
      setAdding(false);
    }
  };

  const addTorrentFile = async () => {
    if (!engine || !connected || adding) return;
    setAdding(true);
    try {
      const filePath = await invoke<string | null>("choose_torrent_file");
      if (!filePath) return;
      await invoke<string>("add_torrent_file", {
        filePath,
        targetDir: targetDir.trim() || preferences?.downloadDir || engine.downloadDir,
      });
      setFilter("all");
      announce("Torrent adicionado à fila.");
      await refreshData();
    } catch (error) {
      announce(`Não foi possível adicionar o torrent: ${String(error)}`, "error");
    } finally {
      setAdding(false);
    }
  };

  const pasteFromClipboard = async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (!text.trim()) {
        announce("A área de transferência está vazia.", "info");
        return;
      }
      setLinkText((current) => current.trim() ? `${current.trim()}\n${text.trim()}` : text.trim());
      setAdvancedAddOpen(true);
      announce("Link(s) colado(s). Confira antes de adicionar.");
    } catch {
      announce("O acesso à área de transferência foi bloqueado. Use Ctrl+V no campo de link.", "info");
    }
  };

  const chooseTargetDirectory = async (forSettings: boolean) => {
    const current = forSettings ? draftPreferences?.downloadDir ?? preferences?.downloadDir ?? "" : targetDir || preferences?.downloadDir || "";
    try {
      const selected = await invoke<string>("choose_directory", { initialDir: current });
      if (forSettings && draftPreferences) setDraftPreferences({ ...draftPreferences, downloadDir: selected });
      else setTargetDir(selected);
    } catch (error) {
      const message = String(error);
      if (!message.toLowerCase().includes("cancelada")) announce(message, "error");
    }
  };

  const chooseDefaultDirectory = async () => {
    if (!preferences) return;
    try {
      const selected = await invoke<string>("choose_directory", { initialDir: preferences.downloadDir });
      const updated = await invoke<Preferences>("update_preferences", {
        preferences: { ...preferences, downloadDir: selected },
      });
      setPreferences(updated);
      setDraftPreferences((current) => current ? { ...current, downloadDir: updated.downloadDir } : current);
      announce("Pasta padrão alterada. Os próximos downloads usarão esse destino.");
    } catch (error) {
      const message = String(error);
      if (!message.toLowerCase().includes("cancelada")) announce(`Não consegui alterar a pasta: ${message}`, "error");
    }
  };

  const downloadAction = async (item: DownloadItem, action: "pause" | "resume" | "remove" | "retry") => {
    if (action === "remove" && ["active", "waiting", "paused"].includes(item.status)) {
      if (!window.confirm(`Remover “${item.name}” da fila? O arquivo parcial será mantido no disco.`)) return;
    }
    setBusyGid(item.gid);
    try {
      if (action === "pause") await invoke("pause_download", { gid: item.gid });
      else if (action === "resume") await invoke("resume_download", { gid: item.gid });
      else if (action === "retry") await invoke("retry_download", { sourceUrl: item.sourceUrl, fileName: item.name, targetDir: item.directory || preferences?.downloadDir || "" });
      else await invoke("remove_download", { gid: item.gid, status: item.status });
      announce(action === "retry" ? "Nova tentativa adicionada à fila." : action === "remove" ? "Download removido." : action === "pause" ? "Download pausado." : "Download retomado.");
      await refreshData();
    } catch (error) {
      announce(`Ação não concluída: ${String(error)}`, "error");
    } finally {
      setBusyGid(null);
    }
  };

  const pauseOrResumeAll = async () => {
    try {
      if (activeCount > 0 || waitingCount > 0) {
        await invoke("pause_all");
        announce("Fila pausada.");
      } else {
        await invoke("resume_all");
        announce("Fila retomada.");
      }
      await refreshData();
    } catch (error) {
      announce(`Não foi possível alterar a fila: ${String(error)}`, "error");
    }
  };

  const clearFinished = async () => {
    const finishedCount = completedCount + failedCount;
    if (!finishedCount) {
      announce("O histórico já está limpo.", "info");
      return;
    }
    if (!window.confirm(`Limpar ${finishedCount} item(ns) concluído(s) ou com falha do histórico? Os arquivos no disco não serão apagados.`)) return;
    try {
      const count = await invoke<number>("clear_finished");
      announce(`Histórico limpo (${count} registro(s)).`);
      await refreshData();
    } catch (error) {
      announce(`Não foi possível limpar o histórico: ${String(error)}`, "error");
    }
  };

  const openPath = async (path: string, reveal: boolean) => {
    if (!path) {
      announce("O caminho do arquivo ainda não está disponível.", "info");
      return;
    }
    try {
      await invoke("open_path", { path, reveal });
    } catch (error) {
      announce(String(error), "error");
    }
  };

  const copyLink = async (raw: string) => {
    if (!raw) {
      announce("O aria2 não expôs o link original desse item.", "info");
      return;
    }
    try {
      await navigator.clipboard.writeText(raw);
      announce("Link copiado para a área de transferência.");
    } catch {
      announce("Não consegui copiar automaticamente esse link.", "error");
    }
  };

  const openSettings = () => {
    if (preferences) setDraftPreferences({ ...preferences });
    setSettingsOpen(true);
  };

  const applyProfile = (profile: "eco" | "balanced" | "turbo") => {
    if (!draftPreferences) return;
    setDraftPreferences({ ...draftPreferences, ...PROFILES[profile], profile, autoProfile: false });
  };

  const applyAutomaticProfile = () => {
    if (!draftPreferences) return;
    setDraftPreferences({ ...draftPreferences, autoProfile: true });
  };

  const saveSettings = async () => {
    if (!draftPreferences || savingSettings) return;
    setSavingSettings(true);
    try {
      const updated = await invoke<Preferences>("update_preferences", { preferences: draftPreferences });
      setPreferences(updated);
      setDraftPreferences(updated);
      setSettingsOpen(false);
      announce("Preferências salvas e aplicadas ao motor.");
      await refreshData();
    } catch (error) {
      announce(`Não consegui salvar as preferências: ${String(error)}`, "error");
    } finally {
      setSavingSettings(false);
    }
  };

  if (startError) {
    return (
      <main className="startup-screen">
        <div className="brand-mark large"><img src="/pulso-mark.svg" alt="Pulso" /></div>
        <p className="eyebrow">PULSO / RECUPERAÇÃO DO MOTOR</p>
        <h1>O motor não conseguiu iniciar.</h1>
        <p className="startup-copy">O Pulso precisa do aria2 instalado e de uma pasta de downloads acessível. O detalhe abaixo ajuda a encontrar a causa exata.</p>
        <code className="terminal-line">sudo pacman -S --needed aria2</code>
        <details className="error-details" open>
          <summary>Detalhes técnicos</summary>
          <pre>{startError}</pre>
        </details>
        <button className="primary-button" onClick={() => window.location.reload()}><RefreshCw size={16} /> Tentar novamente</button>
      </main>
    );
  }

  const pageTitle = filter === "all" ? "Visão geral" : filter === "active" ? "Em andamento" : filter === "waiting" ? "Na fila" : filter === "paused" ? "Pausados" : filter === "complete" ? "Concluídos" : "Com problemas";

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand-row">
          <div className="brand-mark"><img src="/pulso-mark.svg" alt="" /></div>
          <div className="brand-word">pulso<span>.</span><small>DOWNLOAD STUDIO</small></div>
        </div>

        <div className="side-label">BIBLIOTECA</div>
        <nav className="side-nav" aria-label="Navegação principal">
          <button className={`nav-item ${filter === "all" ? "selected" : ""}`} onClick={() => setFilter("all")}>
            <Gauge size={18} /><span>Visão geral</span><span className="nav-count">{downloads.length}</span>
          </button>
          <button className={`nav-item ${filter === "active" ? "selected" : ""}`} onClick={() => setFilter("active")}>
            <ArrowDownToLine size={18} /><span>Em andamento</span><span className="nav-count">{activeCount}</span>
          </button>
          <button className={`nav-item ${filter === "waiting" ? "selected" : ""}`} onClick={() => setFilter("waiting")}>
            <Clock3 size={18} /><span>Na fila</span><span className="nav-count">{waitingCount}</span>
          </button>
          <button className={`nav-item ${filter === "paused" ? "selected" : ""}`} onClick={() => setFilter("paused")}>
            <Pause size={18} /><span>Pausados</span><span className="nav-count">{pausedCount}</span>
          </button>
          <button className={`nav-item ${filter === "complete" ? "selected" : ""}`} onClick={() => setFilter("complete")}>
            <CheckCircle2 size={18} /><span>Concluídos</span><span className="nav-count">{completedCount}</span>
          </button>
          <button className={`nav-item ${filter === "error" ? "selected" : ""}`} onClick={() => setFilter("error")}>
            <AlertCircle size={18} /><span>Com problemas</span><span className="nav-count">{failedCount}</span>
          </button>
        </nav>

        <div className="sidebar-section-label">FERRAMENTAS</div>
        <button className="nav-item" onClick={() => { setAdvancedAddOpen(true); document.getElementById("download-links")?.focus(); }}>
          <ClipboardPaste size={18} /><span>Adicionar em lote</span><span className="nav-trailing"><Plus size={14} /></span>
        </button>
        <button className="nav-item" onClick={() => void clearFinished()}>
          <Trash2 size={18} /><span>Limpar histórico</span>
        </button>

        <div className="sidebar-spacer" />
        <div className="engine-card">
          <div className="engine-card-top"><span className={`status-dot ${connected ? "" : "offline"}`} /><span>{connected ? "Motor conectado" : engine ? "Reconectando…" : "Inicializando"}</span><ShieldCheck size={16} /></div>
          <div className="engine-version">aria2 {engine?.aria2Version ?? "…"}</div>
          <div className="engine-note">RPC local · token privado</div>
          <div className="engine-subline"><span>Última sync</span><strong>{lastSync ? lastSync.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit", second: "2-digit" }) : "—"}</strong></div>
        </div>
        <button className="nav-item bottom-nav" onClick={openSettings}><Settings2 size={18} /><span>Preferências</span><ChevronRight size={15} /></button>
        <div className="sidebar-footer"><span className="footer-indicator" /> Linux native <span className="footer-version">v{engine?.appVersion ?? "0.2.0"}</span></div>
      </aside>

      <main className="main-panel">
        <header className="topbar">
          <div className="breadcrumb"><span>Seu espaço</span><ChevronRight size={14} /><strong>{pageTitle}</strong></div>
          <div className="topbar-actions">
            <div className={`connection-pill ${connected ? "" : "connection-offline"}`}><span className="status-dot" /> {connected ? "Online" : "Sem resposta"}</div>
            <button className="icon-button" title="Atualizar agora" onClick={() => { setRefreshing(true); void refreshData().finally(() => setRefreshing(false)); }}><RefreshCw size={16} className={refreshing ? "spin" : ""} /></button>
            <button className="folder-button" onClick={() => void openPath(preferences?.downloadDir ?? engine?.downloadDir ?? "", false)}><FolderOpen size={16} /> Abrir destino</button>
          </div>
        </header>

        <div className="content-area">
          <section className="welcome-row">
            <div>
              <div className="eyebrow"><Sparkles size={13} /> CONTROLE FINO. MENOS ATRITO.</div>
              <h1>O ritmo dos seus <span>downloads.</span></h1>
              <p className="subtitle">Links em lote, fila persistente e desempenho ajustável — tudo no mesmo lugar.</p>
            </div>
            <div className="hero-status">
              <div className="hero-orbits"><div className="hero-orbit orbit-one" /><div className="hero-orbit orbit-two" /><div className="hero-icon"><Activity size={27} /></div><span className="hero-wave wave-one" /><span className="hero-wave wave-two" /></div>
              <div><strong>{connected ? "PULSO ATIVO" : "MOTOR OFFLINE"}</strong><small>{liveCount} transferências na sessão</small></div>
            </div>
          </section>

          <form className={`add-panel ${advancedAddOpen ? "add-panel-expanded" : ""}`} onSubmit={submitDownload}>
            <div className="add-panel-heading">
              <div className="add-panel-icon"><Link2 size={19} /></div>
              <div className="url-input-wrap">
                <label htmlFor="download-links">ADICIONAR DOWNLOAD</label>
                <textarea
                  id="download-links"
                  rows={advancedAddOpen ? 3 : 1}
                  spellCheck={false}
                  placeholder="Cole um link, magnet ou vários links (um por linha)… Use o ícone de arquivo para abrir .torrent."
                  value={linkText}
                  onChange={(event) => setLinkText(event.target.value)}
                  onKeyDown={(event) => { if ((event.ctrlKey || event.metaKey) && event.key === "Enter") { event.preventDefault(); const form = event.currentTarget.form; form?.requestSubmit(); } }}
                  autoComplete="off"
                />
                <div className="input-subline"><span className="tiny-green-dot" /> {parsedLinks.length ? `${parsedLinks.length} link(s) reconhecido(s)` : "Multiconexão pronta · pressione Ctrl+Enter para adicionar"}</div>
              </div>
              <div className="add-panel-tools">
                <button className="icon-button" type="button" title="Adicionar arquivo .torrent" aria-label="Adicionar arquivo .torrent" disabled={adding || !engine || !connected} onClick={() => void addTorrentFile()}><FileArchive size={16} /></button>
                <button className="icon-button" type="button" title="Colar da área de transferência" onClick={() => void pasteFromClipboard()}><ClipboardPaste size={16} /></button>
                <button className="icon-button" type="button" title={advancedAddOpen ? "Recolher opções" : "Opções de download"} onClick={() => setAdvancedAddOpen((value) => !value)}><SlidersHorizontal size={16} /></button>
              </div>
              <button className="primary-button add-button" type="submit" disabled={!parsedLinks.length || adding || !engine || !connected}>
                {adding ? <LoaderCircle size={17} className="spin" /> : <Plus size={18} />} {adding ? "Na fila…" : parsedLinks.length > 1 ? `Adicionar ${parsedLinks.length}` : "Adicionar"}
              </button>
            </div>
            <div className="default-destination-row">
              <div className="destination-leading"><FolderOpen size={15} /><div><small>PASTA PADRÃO</small><strong title={preferences?.downloadDir ?? engine?.downloadDir ?? ""}>{preferences?.downloadDir ?? engine?.downloadDir ?? "~/Downloads"}</strong></div></div>
              <div className="destination-actions">
                <button className="folder-button" type="button" onClick={() => void chooseDefaultDirectory()}><FolderOpen size={14} /> Alterar pasta</button>
                <button className="icon-button compact-button" type="button" title="Abrir pasta padrão" aria-label="Abrir pasta padrão" onClick={() => void openPath(preferences?.downloadDir ?? engine?.downloadDir ?? "", false)}><ExternalLink size={14} /></button>
              </div>
            </div>
            {advancedAddOpen && (
              <div className="add-advanced-grid">
                <label className="field-control"><span>Nome do arquivo <small>(opcional, apenas um link)</small></span><input value={fileName} onChange={(event) => setFileName(event.target.value)} placeholder="ex.: arquivo-final.zip" /></label>
                <div className="field-control"><span>Salvar em</span><div className="directory-select"><div className="directory-display"><FolderOpen size={14} /> <span>{targetDir || preferences?.downloadDir || "~/Downloads"}</span></div><button className="folder-button" type="button" onClick={() => void chooseTargetDirectory(false)}>Escolher</button>{targetDir && <button className="icon-button compact-button" type="button" title="Usar pasta padrão" onClick={() => setTargetDir("")}><X size={14} /></button>}</div></div>
                <div className="batch-tip"><CircleHelp size={15} /><span>Vários links são criados como downloads independentes. Linhas repetidas são ignoradas.</span></div>
              </div>
            )}
          </form>

          <section className="stats-grid" aria-label="Estatísticas de transferência">
            <div className="stat-card speed-card">
              <div className="stat-topline"><div className="stat-icon download-stat"><ArrowDownToLine size={18} /></div><span className="stat-label">VELOCIDADE DE DOWNLOAD</span><span className="live-badge"><span /> AO VIVO</span></div>
              <div className="stat-value">{formatSpeed(stats.downloadSpeed, preferences?.speedUnit ?? "MB")}</div>
              <div className="chart-wrap"><SpeedSparkline values={speedHistory} /></div>
              <div className="stat-foot"><span className="stat-foot-icon"><Activity size={13} /></span> Histórico dos últimos ~40 segundos</div>
            </div>
            <div className="stat-card">
              <div className="stat-topline"><div className="stat-icon upload-stat"><ArrowUpFromLine size={18} /></div><span className="stat-label">VELOCIDADE DE UPLOAD</span></div>
              <div className="stat-value">{formatSpeed(stats.uploadSpeed, preferences?.speedUnit ?? "MB")}</div>
              <div className="stat-detail-line"><span className="neutral-dot" /> Tráfego de saída do motor</div>
              <div className="stat-bottom-line"><span>Limite</span><strong>{preferences?.maxUploadSpeed === "0" || !preferences ? "Sem limite" : `${preferences.maxUploadSpeed}/s`}</strong></div>
            </div>
            <div className="stat-card queue-card">
              <div className="stat-topline"><div className="stat-icon queue-stat"><LayersIcon /></div><span className="stat-label">FILA INTELIGENTE</span></div>
              <div className="stat-value">{liveCount}<span className="stat-unit"> itens na fila ou pausados</span></div>
              <div className="stack-progress"><span className="segment-active" style={{ width: `${liveCount ? (activeCount / liveCount) * 100 : 0}%` }} /><span className="segment-waiting" style={{ width: `${liveCount ? (waitingCount / liveCount) * 100 : 0}%` }} /><span className="segment-paused" style={{ width: `${liveCount ? (pausedCount / liveCount) * 100 : 0}%` }} /></div>
              <div className="stat-foot stat-foot-split"><span><i className="legend-active" />{activeCount} baixando</span><span><i className="legend-waiting" />{waitingCount} fila</span><span><i className="legend-paused" />{pausedCount} pausado(s)</span></div>
              {activeCount > 0 && etaSeconds !== null && <div className="queue-eta">Fim estimado da fila: ~{formatDuration(etaSeconds)} <small>itens com tamanho conhecido · ativos + fila</small></div>}
            </div>
          </section>

          <section className="downloads-section">
            <div className="section-heading">
              <div><div className="section-title-line"><h2>{filter === "all" ? "Sua biblioteca" : pageTitle}</h2><span className="count-bubble">{filteredDownloads.length}</span></div><p>Controles por arquivo, retentativa de falhas e histórico que sobrevive ao fechamento.</p></div>
              <div className="list-actions">
                <label className="search-box"><Search size={16} /><input type="search" placeholder="Buscar arquivo ou origem…" value={search} onChange={(event) => setSearch(event.target.value)} /></label>
                <button className="subtle-button" onClick={() => void pauseOrResumeAll()} title="Pausar ou retomar toda a fila">{activeCount > 0 || waitingCount > 0 ? <Pause size={15} /> : <Play size={15} />}{activeCount > 0 || waitingCount > 0 ? "Pausar fila" : "Retomar fila"}</button>
                <button className="square-button" onClick={() => void clearFinished()} title="Limpar histórico de concluídos e falhas"><Trash2 size={15} /></button>
              </div>
            </div>

            {!engine ? (
              <div className="loading-state"><LoaderCircle size={25} className="spin" /><span>Preparando o motor…</span></div>
            ) : filteredDownloads.length === 0 ? (
              <div className="empty-state">
                <div className="empty-illustration"><div className="empty-ring ring-a" /><div className="empty-ring ring-b" /><div className="empty-core"><Download size={28} /></div><span className="floating-dot dot-a" /><span className="floating-dot dot-b" /></div>
                <h3>{search ? "Nenhum resultado" : filter === "complete" ? "Ainda não há downloads concluídos" : filter === "error" ? "Tudo certo por aqui" : filter === "paused" ? "Nenhum download pausado" : "Dê o primeiro pulso"}</h3>
                <p>{search ? "Tente outro nome, identificador ou domínio de origem." : "Cole um link acima ou adicione vários de uma vez. O Pulso cuida da fila e da retomada."}</p>
                {!search && filter !== "all" && <button className="text-button" onClick={() => setFilter("all")}>Ver toda a biblioteca <ChevronRight size={14} /></button>}
                {!search && filter === "all" && <button className="text-button" onClick={() => { setAdvancedAddOpen(true); document.getElementById("download-links")?.focus(); }}>Adicionar meu primeiro link <ChevronRight size={14} /></button>}
              </div>
            ) : (
              <div className="download-list">
                {filteredDownloads.map((item) => {
                  const percent = formatPercent(item.completedLength, item.totalLength);
                  const paused = item.status === "paused";
                  const canPause = ["active", "waiting"].includes(item.status);
                  const canResume = paused;
                  const terminal = ["complete", "error", "removed"].includes(item.status);
                  const hasFile = Boolean(item.filePath);
                  const remainingBytes = Math.max(0, item.totalLength - item.completedLength);
                  const smoothedRate = smoothedRates[item.gid] ?? 0;
                  const eta = item.status === "paused" ? "Pausado" : item.status === "waiting" ? "Na fila" : item.status === "active" && item.totalLength <= 0 ? "Descobrindo tamanho…" : item.status === "active" && smoothedRate > 1 && remainingBytes > 0 ? formatDuration(remainingBytes / smoothedRate) : item.status === "active" && remainingBytes === 0 && item.totalLength > 0 ? "Finalizando…" : "Calculando ritmo…";
                  return (
                    <article className={`download-row ${item.status === "error" ? "has-error" : ""} ${item.status === "complete" ? "is-complete" : ""}`} key={item.gid}>
                      <div className={`file-type-icon ${item.status === "complete" ? "complete-file" : item.status === "error" ? "error-file" : ""}`}>{item.status === "complete" ? <Check size={18} /> : item.status === "error" ? <AlertCircle size={18} /> : fileIcon(item.name)}</div>
                      <div className="file-main">
                        <div className="file-title-line"><h3 title={item.name}>{item.name}</h3><span className={`status-label status-${item.status}`}>{item.status === "active" && <span className="status-pulse" />}{statusText(item.status)}</span></div>
                        <div className="file-meta"><span>{formatBytes(item.completedLength)} <span className="meta-divider">/</span> {item.totalLength > 0 ? formatBytes(item.totalLength) : "tamanho desconhecido"}</span><span className="meta-divider">·</span><span className="source-text" title={item.sourceUrl}>{sourceLabel(item.sourceUrl)}</span><span className="meta-divider">·</span><span>{item.status === "active" ? formatSpeed(item.downloadSpeed, preferences?.speedUnit ?? "MB") : item.status === "complete" ? "Arquivo pronto" : item.status === "error" ? "Precisa de atenção" : item.status === "paused" ? "Em espera" : item.status === "waiting" ? "Aguardando conexão" : statusText(item.status)}</span>{item.status === "active" && item.connections > 0 && <><span className="meta-divider">·</span><span>{item.connections} conexões</span></>}</div>
                        <div className={`progress-track ${item.status === "complete" ? "progress-complete" : item.status === "error" ? "progress-error" : ""}`}><div className={`progress-fill ${item.status === "active" ? "progress-animated" : ""}`} style={{ width: `${item.status === "complete" ? 100 : percent}%` }} /></div>
                        <div className="file-submeta">{item.status === "active" ? <><span><Gauge size={12} /> {item.totalLength <= 0 || eta === "Calculando ritmo…" ? eta : `${eta} restantes`}</span><span><HardDrive size={12} /> {item.directory || preferences?.downloadDir}</span>{item.tuningProfile && <span className="tuning-badge" title="Estratégia de conexões aplicada a este download"><Sparkles size={11} /> {item.tuningProfile}</span>}</> : item.status === "complete" ? <><span><CheckCheck size={12} /> Concluído · disponível no disco</span>{item.tuningProfile && <span className="tuning-badge"><Sparkles size={11} /> {item.tuningProfile}</span>}</> : item.status === "error" ? <span className="error-reason" title={item.errorMessage}><AlertCircle size={12} /> {item.errorMessage || "A origem não concluiu a transferência."}</span> : <><span><FolderOpen size={12} /> {item.directory || preferences?.downloadDir}</span>{item.tuningProfile && <span className="tuning-badge"><Sparkles size={11} /> {item.tuningProfile}</span>}</>}</div>
                      </div>
                      <div className="download-percent">{item.status === "complete" ? <CheckCircle2 size={18} /> : item.status === "error" ? <AlertCircle size={18} /> : item.totalLength > 0 ? `${Math.round(percent)}%` : <span className="unknown-progress">···</span>}</div>
                      <div className="row-action-group">
                        {canPause && <button className="row-action" disabled={busyGid === item.gid} title="Pausar" aria-label={`Pausar ${item.name}`} onClick={() => void downloadAction(item, "pause")}><Pause size={15} /></button>}
                        {canResume && <button className="row-action resume-action" disabled={busyGid === item.gid} title="Retomar" aria-label={`Retomar ${item.name}`} onClick={() => void downloadAction(item, "resume")}><Play size={15} /></button>}
                        {item.status === "error" && <button className="row-action retry-action" disabled={busyGid === item.gid || !item.sourceUrl} title="Tentar novamente" aria-label={`Tentar novamente ${item.name}`} onClick={() => void downloadAction(item, "retry")}><RotateCcw size={15} /></button>}
                        {item.status === "complete" && hasFile && <button className="row-action open-action" title="Abrir arquivo" aria-label={`Abrir ${item.name}`} onClick={() => void openPath(item.filePath, false)}><ExternalLink size={15} /></button>}
                        {hasFile && <button className="row-action" title="Mostrar na pasta" aria-label={`Mostrar ${item.name} na pasta`} onClick={() => void openPath(item.filePath, true)}><FolderOpen size={15} /></button>}
                        {item.sourceUrl && !item.sourceUrl.startsWith("torrent-file:") && <button className="row-action" title="Copiar link original" aria-label={`Copiar link de ${item.name}`} onClick={() => void copyLink(item.sourceUrl)}><Copy size={14} /></button>}
                        {terminal && <button className="row-action remove-action" disabled={busyGid === item.gid} title="Remover do histórico" aria-label={`Remover ${item.name}`} onClick={() => void downloadAction(item, "remove")}><X size={15} /></button>}
                      </div>
                    </article>
                  );
                })}
              </div>
            )}
          </section>

          <footer className="main-footer"><span><ShieldCheck size={14} /> RPC restrito ao computador e protegido por token</span><span><Wifi size={14} /> Velocidade real depende do servidor e da rede</span><span><Zap size={14} /> Feito para o desktop Linux</span></footer>
        </div>
      </main>

      {settingsOpen && draftPreferences && (
        <div className="modal-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) setSettingsOpen(false); }}>
          <section className="settings-modal" role="dialog" aria-modal="true" aria-labelledby="settings-title">
            <div className="modal-heading"><div className="settings-icon"><Settings2 size={19} /></div><div><h2 id="settings-title">Preferências do Pulso</h2><p>Essas opções são aplicadas ao motor e salvas neste computador.</p></div><button className="icon-button modal-close" onClick={() => setSettingsOpen(false)} title="Fechar" aria-label="Fechar preferências"><X size={17} /></button></div>
            <div className="settings-scroll">
              <div className="settings-block">
                <div className="setting-heading"><div><span>Perfil de desempenho</span><small>Escolhe um preset ou ajusta os limites manualmente.</small></div><span className="setting-chip"><Zap size={12} /> {draftPreferences.autoProfile ? "PULSO AUTO" : draftPreferences.profile === "eco" ? "ECONÔMICO" : draftPreferences.profile === "balanced" ? "EQUILIBRADO" : draftPreferences.profile === "turbo" ? "TURBO" : "PERSONALIZADO"}</span></div>
                <div className="profile-grid">
                  <button className={`profile-card ${draftPreferences.autoProfile ? "selected" : ""}`} onClick={applyAutomaticProfile}><span className="profile-icon automatic"><Sparkles size={18} /></span><strong>Pulso Auto</strong><small>Analisa tamanho, protocolo e pistas de suporte a partes.</small><span className="profile-numbers">Ajuste por download</span></button>
                  <button className={`profile-card ${!draftPreferences.autoProfile && draftPreferences.profile === "eco" ? "selected" : ""}`} onClick={() => applyProfile("eco")}><span className="profile-icon eco"><Activity size={18} /></span><strong>Economia</strong><small>Menos conexões e impacto menor na rede.</small><span className="profile-numbers">3 downloads · 4 conexões</span></button>
                  <button className={`profile-card ${!draftPreferences.autoProfile && draftPreferences.profile === "balanced" ? "selected" : ""}`} onClick={() => applyProfile("balanced")}><span className="profile-icon balanced"><Gauge size={18} /></span><strong>Equilibrado</strong><small>Bom ponto de partida para uso diário.</small><span className="profile-numbers">5 downloads · 8 conexões</span></button>
                  <button className={`profile-card ${!draftPreferences.autoProfile && draftPreferences.profile === "turbo" ? "selected" : ""}`} onClick={() => applyProfile("turbo")}><span className="profile-icon turbo"><Zap size={18} /></span><strong>Turbo</strong><small>Mais paralelismo quando a origem permite.</small><span className="profile-numbers">8 downloads · 16 conexões</span></button>
                </div>
                <p className="settings-note auto-profile-note"><Sparkles size={14} /> No modo automático, o Pulso usa o tamanho informado pela origem quando disponível, faz fallback pelo tipo de link/arquivo e reduz para uma conexão se o servidor declarar que não aceita intervalos. Isso não garante mais velocidade em todas as origens.</p>
              </div>
              <div className="settings-block">
                <div className="setting-heading"><div><span>Pasta padrão</span><small>Novos downloads usam este destino, salvo ajuste por download.</small></div></div>
                <div className="directory-select settings-directory"><div className="directory-display"><FolderOpen size={15} /><span>{draftPreferences.downloadDir}</span></div><button className="folder-button" onClick={() => void chooseTargetDirectory(true)}>Escolher pasta</button></div>
              </div>
              <div className="settings-block">
                <div className="setting-heading"><div><span>Conexões e fila</span><small>Valores altos nem sempre são mais rápidos.</small></div></div>
                <div className="settings-fields">
                  <label className="field-control"><span>Downloads simultâneos</span><select value={draftPreferences.maxConcurrentDownloads} onChange={(event) => setDraftPreferences({ ...draftPreferences, maxConcurrentDownloads: Number(event.target.value), profile: "custom", autoProfile: false })}>{[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((value) => <option key={value} value={value}>{value} {value === 1 ? "download" : "downloads"}</option>)}</select></label>
                  <label className="field-control"><span>Conexões por servidor</span><select value={draftPreferences.connectionsPerServer} onChange={(event) => setDraftPreferences({ ...draftPreferences, connectionsPerServer: Number(event.target.value), profile: "custom", autoProfile: false })}>{[1, 2, 4, 8, 16].map((value) => <option key={value} value={value}>{value} conexões</option>)}</select></label>
                  <label className="field-control"><span>Divisões por arquivo</span><select value={draftPreferences.split} onChange={(event) => setDraftPreferences({ ...draftPreferences, split: Number(event.target.value), profile: "custom", autoProfile: false })}>{[1, 2, 4, 8, 16].map((value) => <option key={value} value={value}>{value} partes</option>)}</select></label>
                  <label className="field-control"><span>Parte mínima</span><select value={draftPreferences.minSplitSize} onChange={(event) => setDraftPreferences({ ...draftPreferences, minSplitSize: event.target.value, profile: "custom", autoProfile: false })}>{[{ value: "1M", label: "1 MB" }, { value: "2M", label: "2 MB" }, { value: "4M", label: "4 MB" }].map((item) => <option key={item.value} value={item.value}>{item.label}</option>)}</select></label>
                </div>
              </div>
              <div className="settings-block">
                <div className="setting-heading"><div><span>Limites de velocidade</span><small>Zero significa sem limite imposto pelo Pulso.</small></div></div>
                <div className="settings-fields">
                  <label className="field-control"><span>Limite de download</span><select value={draftPreferences.maxDownloadSpeed} onChange={(event) => setDraftPreferences({ ...draftPreferences, maxDownloadSpeed: event.target.value })}>{[{ value: "0", label: "Sem limite" }, { value: "256K", label: "256 KB/s" }, { value: "512K", label: "512 KB/s" }, { value: "1M", label: "1 MB/s" }, { value: "2M", label: "2 MB/s" }, { value: "5M", label: "5 MB/s" }, { value: "10M", label: "10 MB/s" }, { value: "25M", label: "25 MB/s" }, { value: "50M", label: "50 MB/s" }, { value: "100M", label: "100 MB/s" }].map((item) => <option key={item.value} value={item.value}>{item.label}</option>)}</select></label>
                  <label className="field-control"><span>Limite de upload</span><select value={draftPreferences.maxUploadSpeed} onChange={(event) => setDraftPreferences({ ...draftPreferences, maxUploadSpeed: event.target.value })}>{[{ value: "0", label: "Sem limite" }, { value: "256K", label: "256 KB/s" }, { value: "512K", label: "512 KB/s" }, { value: "1M", label: "1 MB/s" }, { value: "2M", label: "2 MB/s" }, { value: "5M", label: "5 MB/s" }, { value: "10M", label: "10 MB/s" }, { value: "25M", label: "25 MB/s" }, { value: "50M", label: "50 MB/s" }, { value: "100M", label: "100 MB/s" }].map((item) => <option key={item.value} value={item.value}>{item.label}</option>)}</select></label>
                  <label className="field-control"><span>Unidade de velocidade</span><select value={draftPreferences.speedUnit} onChange={(event) => setDraftPreferences({ ...draftPreferences, speedUnit: event.target.value })}><option value="MB">Megabytes por segundo (MB/s)</option><option value="Mb">Megabits por segundo (Mb/s)</option></select></label>
                </div>
                <label className="notification-setting"><input type="checkbox" checked={draftPreferences.notifyOnComplete} onChange={(event) => setDraftPreferences({ ...draftPreferences, notifyOnComplete: event.target.checked })} /><span><strong>Notificações do desktop</strong><small>Avisar pelo KDE quando um download terminar. Requer o serviço de notificações do sistema.</small></span><CheckCheck size={17} /></label>
                <p className="settings-note"><ShieldCheck size={14} /> Certificados HTTPS permanecem verificados. O Pulso não desativa a validação TLS para ganhar velocidade.</p>
              </div>
            </div>
            <div className="modal-footer"><span><HardDrive size={14} /> Preferências persistentes</span><div><button className="subtle-button" onClick={() => setSettingsOpen(false)}>Cancelar</button><button className="primary-button" onClick={() => void saveSettings()} disabled={savingSettings}>{savingSettings ? <LoaderCircle size={15} className="spin" /> : <Check size={15} />} Salvar alterações</button></div></div>
          </section>
        </div>
      )}

      {toast && <div className={`toast toast-${toast.kind}`} role="status"><span className="toast-icon">{toast.kind === "error" ? <AlertCircle size={17} /> : toast.kind === "info" ? <CircleHelp size={17} /> : <CheckCircle2 size={17} />}</span><span>{toast.message}</span><button onClick={() => setToast(null)} aria-label="Fechar notificação"><X size={14} /></button></div>}
    </div>
  );
}

function LayersIcon() {
  return <HardDrive size={18} />;
}

export default App;
