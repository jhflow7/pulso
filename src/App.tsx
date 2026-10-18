import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  AlertCircle,
  ArrowDownToLine,
  ArrowUpFromLine,
  Check,
  CheckCircle2,
  ChevronRight,
  Clock3,
  Download,
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
  Search,
  Settings2,
  ShieldCheck,
  Sparkles,
  Trash2,
  Wifi,
  X,
  Zap,
} from "lucide-react";

type DownloadItem = {
  gid: string;
  name: string;
  status: string;
  totalLength: number;
  completedLength: number;
  downloadSpeed: number;
  uploadSpeed: number;
  connections: number;
  errorMessage: string;
  directory: string;
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
};

type Filter = "all" | "active" | "waiting" | "complete" | "error";

const zeroStats: Stats = {
  downloadSpeed: 0,
  uploadSpeed: 0,
  active: 0,
  waiting: 0,
  stopped: 0,
};

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const amount = bytes / Math.pow(1024, index);
  return `${amount.toLocaleString("pt-BR", { maximumFractionDigits: amount >= 100 ? 0 : 1 })} ${units[index]}`;
}

function formatSpeed(bytes: number): string {
  return bytes > 0 ? `${formatBytes(bytes)}/s` : "0 KB/s";
}

function formatPercent(done: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(100, Math.max(0, (done / total) * 100));
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
  if (["zip", "7z", "rar", "tar", "gz", "xz", "iso"].includes(extension)) return <FileArchive size={18} />;
  if (["jpg", "jpeg", "png", "webp", "gif", "svg", "mp4", "mkv"].includes(extension)) return <FileImage size={18} />;
  if (["mp3", "flac", "wav", "ogg", "opus"].includes(extension)) return <FileAudio size={18} />;
  if (["pdf", "txt", "md", "doc", "docx", "epub"].includes(extension)) return <FileText size={18} />;
  return <Download size={18} />;
}

function App() {
  const [engine, setEngine] = useState<EngineInfo | null>(null);
  const [startError, setStartError] = useState("");
  const [downloads, setDownloads] = useState<DownloadItem[]>([]);
  const [stats, setStats] = useState<Stats>(zeroStats);
  const [filter, setFilter] = useState<Filter>("all");
  const [url, setUrl] = useState("");
  const [search, setSearch] = useState("");
  const [adding, setAdding] = useState(false);
  const [busyGid, setBusyGid] = useState<string | null>(null);
  const [toast, setToast] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

  const refreshData = useCallback(async () => {
    try {
      const [nextDownloads, nextStats] = await Promise.all([
        invoke<DownloadItem[]>("get_downloads"),
        invoke<Stats>("get_stats"),
      ]);
      setDownloads(nextDownloads);
      setStats(nextStats);
    } catch (error) {
      // Pequenas falhas transitórias durante o polling não devem interromper a interface.
      console.warn("Pulso: falha ao atualizar a lista", error);
    }
  }, []);

  useEffect(() => {
    let alive = true;
    let timer: number | undefined;

    const boot = async () => {
      try {
        const info = await invoke<EngineInfo>("initialize");
        if (!alive) return;
        setEngine(info);
        await refreshData();
        if (alive) {
          timer = window.setInterval(() => void refreshData(), 1200);
        }
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
    const timer = window.setTimeout(() => setToast(""), 3200);
    return () => window.clearTimeout(timer);
  }, [toast]);

  const filteredDownloads = useMemo(() => {
    const normalized = search.trim().toLowerCase();
    return downloads.filter((item) => {
      const matchesFilter = filter === "all"
        || (filter === "active" && ["active", "paused"].includes(item.status))
        || (filter === "waiting" && item.status === "waiting")
        || (filter === "complete" && item.status === "complete")
        || (filter === "error" && item.status === "error");
      const matchesSearch = !normalized || item.name.toLowerCase().includes(normalized) || item.gid.includes(normalized);
      return matchesFilter && matchesSearch;
    });
  }, [downloads, filter, search]);

  const activeCount = downloads.filter((item) => item.status === "active").length;
  const waitingCount = downloads.filter((item) => item.status === "waiting").length;
  const completedCount = downloads.filter((item) => item.status === "complete").length;
  const failedCount = downloads.filter((item) => item.status === "error").length;
  const pausedCount = downloads.filter((item) => item.status === "paused").length;
  const queueCount = activeCount + waitingCount + pausedCount;

  const submitDownload = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!url.trim() || adding) return;
    setAdding(true);
    try {
      await invoke<string>("add_download", { url: url.trim() });
      setUrl("");
      setFilter("all");
      setToast("Download adicionado à fila.");
      await refreshData();
    } catch (error) {
      setToast(`Não foi possível adicionar: ${String(error)}`);
    } finally {
      setAdding(false);
    }
  };

  const handleDownloadAction = async (item: DownloadItem, action: "pause" | "resume" | "remove") => {
    setBusyGid(item.gid);
    try {
      if (action === "pause") await invoke("pause_download", { gid: item.gid });
      else if (action === "resume") await invoke("resume_download", { gid: item.gid });
      else await invoke("remove_download", { gid: item.gid, status: item.status });
      await refreshData();
    } catch (error) {
      setToast(`Ação não concluída: ${String(error)}`);
    } finally {
      setBusyGid(null);
    }
  };

  const pauseOrResumeAll = async () => {
    try {
      if (activeCount > 0 || waitingCount > 0) {
        await invoke("pause_all");
        setToast("Downloads pausados.");
      } else {
        await invoke("resume_all");
        setToast("Fila retomada.");
      }
      await refreshData();
    } catch (error) {
      setToast(`Não foi possível alterar a fila: ${String(error)}`);
    }
  };

  const clearFinished = async () => {
    try {
      const count = await invoke<number>("clear_finished");
      setToast(`${count} item(ns) removido(s) do histórico.`);
      await refreshData();
    } catch (error) {
      setToast(`Não foi possível limpar o histórico: ${String(error)}`);
    }
  };

  const openFolder = async () => {
    try {
      await invoke("open_download_folder");
    } catch (error) {
      setToast(String(error));
    }
  };

  const doManualRefresh = async () => {
    setRefreshing(true);
    await refreshData();
    setRefreshing(false);
  };

  if (startError) {
    return (
      <main className="startup-screen">
        <div className="brand-mark large"><Activity size={28} /></div>
        <p className="eyebrow">PULSO / PRIMEIRA EXECUÇÃO</p>
        <h1>O motor ainda não respondeu.</h1>
        <p className="startup-copy">O Pulso usa o aria2 como motor de download. Instala ele pelo terminal e abre o app novamente.</p>
        <code className="terminal-line">sudo pacman -S --needed aria2</code>
        <details className="error-details">
          <summary>Detalhes técnicos</summary>
          <pre>{startError}</pre>
        </details>
        <button className="primary-button" onClick={() => window.location.reload()}><RefreshCw size={16} /> Tentar novamente</button>
      </main>
    );
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand-row">
          <div className="brand-mark"><Activity size={21} strokeWidth={2.7} /></div>
          <div className="brand-word">pulso<span>.</span><small>DOWNLOAD MANAGER</small></div>
        </div>

        <div className="side-label">ESPAÇO DE TRABALHO</div>
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
          <button className={`nav-item ${filter === "complete" ? "selected" : ""}`} onClick={() => setFilter("complete")}>
            <CheckCircle2 size={18} /><span>Concluídos</span><span className="nav-count">{completedCount}</span>
          </button>
          <button className={`nav-item ${filter === "error" ? "selected" : ""}`} onClick={() => setFilter("error")}>
            <AlertCircle size={18} /><span>Com problemas</span><span className="nav-count">{failedCount}</span>
          </button>
        </nav>

        <div className="sidebar-spacer" />
        <div className="engine-card">
          <div className="engine-card-top"><span className="status-dot" /><span>Motor conectado</span><ShieldCheck size={16} /></div>
          <div className="engine-version">aria2 {engine?.aria2Version ?? "…"}</div>
          <div className="engine-note">RPC local protegido</div>
        </div>
        <button className="nav-item bottom-nav" onClick={() => setSettingsOpen(true)}><Settings2 size={18} /><span>Preferências</span><ChevronRight size={15} /></button>
        <div className="sidebar-footer"><span className="footer-indicator" /> CachyOS ready <span className="footer-version">v0.1.0</span></div>
      </aside>

      <main className="main-panel">
        <header className="topbar">
          <div className="breadcrumb"><span>Workspace</span><ChevronRight size={14} /><strong>{filter === "all" ? "Visão geral" : filter === "active" ? "Em andamento" : filter === "waiting" ? "Na fila" : filter === "complete" ? "Concluídos" : "Com problemas"}</strong></div>
          <div className="topbar-actions">
            <div className="connection-pill"><span className="status-dot" /> Online</div>
            <button className="icon-button" title="Atualizar downloads" onClick={() => void doManualRefresh()}><RefreshCw size={16} className={refreshing ? "spin" : ""} /></button>
            <button className="folder-button" onClick={() => void openFolder()}><FolderOpen size={16} /> Pasta de downloads</button>
          </div>
        </header>

        <div className="content-area">
          <section className="welcome-row">
            <div>
              <div className="eyebrow"><Sparkles size={13} /> SEU DOWNLOAD, NO RITMO CERTO</div>
              <h1>Baixe sem perder <span>o pulso.</span></h1>
              <p className="subtitle">Tudo no controle. O aria2 faz o trabalho pesado.</p>
            </div>
            <div className="hero-badge"><div className="hero-orbit orbit-one" /><div className="hero-orbit orbit-two" /><div className="hero-icon"><Zap size={25} fill="currentColor" /></div><span>ARIA2 ENGINE</span></div>
          </section>

          <form className="add-panel" onSubmit={submitDownload}>
            <div className="add-panel-icon"><Link2 size={19} /></div>
            <div className="url-input-wrap">
              <label htmlFor="download-url">NOVO DOWNLOAD</label>
              <input id="download-url" type="text" inputMode="url" spellCheck={false} placeholder="Cole um link direto ou magnet link…" value={url} onChange={(event) => setUrl(event.target.value)} autoComplete="url" />
            </div>
            <div className="add-hint"><span className="tiny-green-dot" /> Multiconexão ativada</div>
            <button className="primary-button add-button" type="submit" disabled={!url.trim() || adding || !engine}>
              {adding ? <LoaderCircle size={17} className="spin" /> : <Plus size={18} />} {adding ? "Adicionando" : "Adicionar"}
            </button>
          </form>

          <section className="stats-grid" aria-label="Estatísticas de transferência">
            <div className="stat-card speed-card">
              <div className="stat-topline"><div className="stat-icon download-stat"><ArrowDownToLine size={18} /></div><span className="stat-label">VELOCIDADE DE DOWNLOAD</span><span className="live-badge"><span /> AO VIVO</span></div>
              <div className="stat-value">{formatSpeed(stats.downloadSpeed)}</div>
              <div className="stat-foot"><span className="stat-foot-icon"><Activity size={13} /></span> Atualiza automaticamente <span className="sparkline" aria-hidden="true"><i /><i /><i /><i /><i /><i /><i /><i /><i /><i /><i /><i /></span></div>
            </div>
            <div className="stat-card">
              <div className="stat-topline"><div className="stat-icon upload-stat"><ArrowUpFromLine size={18} /></div><span className="stat-label">VELOCIDADE DE UPLOAD</span></div>
              <div className="stat-value">{formatSpeed(stats.uploadSpeed)}</div>
              <div className="stat-foot"><span className="neutral-dot" /> Tráfego de saída do motor</div>
            </div>
            <div className="stat-card">
              <div className="stat-topline"><div className="stat-icon queue-stat"><LayersIcon /></div><span className="stat-label">NA FILA / ATIVOS</span></div>
              <div className="stat-value">{queueCount}<span className="stat-unit"> downloads</span></div>
              <div className="mini-progress"><span style={{ width: `${queueCount > 0 ? (activeCount / queueCount) * 100 : 0}%` }} /></div>
              <div className="stat-foot">{activeCount} ativos <span className="foot-separator">·</span> {waitingCount} na fila <span className="foot-separator">·</span> {pausedCount} pausados</div>
            </div>
          </section>

          <section className="downloads-section">
            <div className="section-heading">
              <div><div className="section-title-line"><h2>Seus downloads</h2><span className="count-bubble">{filteredDownloads.length}</span></div><p>Acompanhe cada arquivo em tempo real.</p></div>
              <div className="list-actions">
                <label className="search-box"><Search size={16} /><input type="search" placeholder="Buscar arquivo…" value={search} onChange={(event) => setSearch(event.target.value)} /></label>
                <button className="subtle-button" onClick={() => void pauseOrResumeAll()} title="Pausar ou retomar a fila">{activeCount > 0 || waitingCount > 0 ? <Pause size={15} /> : <Play size={15} />}{activeCount > 0 || waitingCount > 0 ? "Pausar fila" : "Retomar fila"}</button>
                <button className="square-button" onClick={() => void clearFinished()} title="Limpar histórico de concluídos e falhas"><Trash2 size={15} /></button>
              </div>
            </div>

            {!engine && !startError ? (
              <div className="loading-state"><LoaderCircle size={25} className="spin" /><span>Conectando ao aria2…</span></div>
            ) : filteredDownloads.length === 0 ? (
              <div className="empty-state">
                <div className="empty-illustration"><div className="empty-ring ring-a" /><div className="empty-ring ring-b" /><div className="empty-core"><Download size={28} /></div><span className="floating-dot dot-a" /><span className="floating-dot dot-b" /></div>
                <h3>{search ? "Nada encontrado" : filter === "complete" ? "Ainda não tem nada concluído" : filter === "error" ? "Nenhum download com problema" : "Sua próxima transferência começa aqui"}</h3>
                <p>{search ? "Tente outro nome ou identificador." : "Cole um link no campo acima e deixa o Pulso cuidar do resto."}</p>
                {!search && filter !== "all" && <button className="text-button" onClick={() => setFilter("all")}>Ver todos os downloads <ChevronRight size={14} /></button>}
              </div>
            ) : (
              <div className="download-list">
                {filteredDownloads.map((item) => {
                  const percent = formatPercent(item.completedLength, item.totalLength);
                  const paused = item.status === "paused";
                  const canPause = ["active", "waiting"].includes(item.status);
                  const canResume = paused;
                  const terminal = ["complete", "error", "removed"].includes(item.status);
                  return (
                    <article className={`download-row ${item.status === "error" ? "has-error" : ""}`} key={item.gid}>
                      <div className={`file-type-icon ${item.status === "complete" ? "complete-file" : ""}`}>{item.status === "complete" ? <Check size={18} /> : fileIcon(item.name)}</div>
                      <div className="file-main">
                        <div className="file-title-line"><h3 title={item.name}>{item.name}</h3><span className={`status-label status-${item.status}`}>{item.status === "active" && <span className="status-pulse" />}{statusText(item.status)}</span></div>
                        <div className="file-meta"><span>{formatBytes(item.completedLength)} <span className="meta-divider">/</span> {item.totalLength > 0 ? formatBytes(item.totalLength) : "tamanho desconhecido"}</span><span className="meta-divider">·</span><span>{item.status === "active" ? formatSpeed(item.downloadSpeed) : item.status === "complete" ? "Arquivo pronto" : item.status === "error" ? "Verifique o link" : item.status === "paused" ? "Transferência pausada" : item.status === "waiting" ? "Esperando uma conexão" : statusText(item.status)}</span>{item.status === "active" && item.connections > 0 && <><span className="meta-divider">·</span><span>{item.connections} conexões</span></>}</div>
                        <div className={`progress-track ${item.status === "complete" ? "progress-complete" : item.status === "error" ? "progress-error" : ""}`}><div className="progress-fill" style={{ width: `${item.status === "complete" ? 100 : percent}%` }} /></div>
                        {item.status === "error" && item.errorMessage && <div className="file-error-message">{item.errorMessage}</div>}
                      </div>
                      <div className="download-percent">{item.status === "complete" ? <CheckCircle2 size={18} /> : item.status === "error" ? <AlertCircle size={18} /> : `${Math.round(percent)}%`}</div>
                      <div className="row-action-group">
                        {canPause && <button className="row-action" disabled={busyGid === item.gid} title="Pausar" onClick={() => void handleDownloadAction(item, "pause")}><Pause size={15} /></button>}
                        {canResume && <button className="row-action resume-action" disabled={busyGid === item.gid} title="Retomar" onClick={() => void handleDownloadAction(item, "resume")}><Play size={15} /></button>}
                        {terminal && item.status !== "removed" && <button className="row-action" disabled={busyGid === item.gid} title="Remover do histórico" onClick={() => void handleDownloadAction(item, "remove")}><X size={15} /></button>}
                        {item.status === "removed" && <button className="row-action" disabled={busyGid === item.gid} title="Remover registro" onClick={() => void handleDownloadAction(item, "remove")}><X size={15} /></button>}
                      </div>
                    </article>
                  );
                })}
              </div>
            )}
          </section>

          <footer className="main-footer"><span><ShieldCheck size={14} /> Conexão local protegida por token</span><span><Wifi size={14} /> O limite real depende do servidor e da sua rede.</span><span>Pulso / feito para o desktop Linux</span></footer>
        </div>
      </main>

      {settingsOpen && (
        <div className="modal-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) setSettingsOpen(false); }}>
          <section className="settings-modal" role="dialog" aria-modal="true" aria-labelledby="settings-title">
            <div className="modal-heading"><div className="settings-icon"><Settings2 size={19} /></div><div><h2 id="settings-title">Preferências</h2><p>Configuração atual do motor aria2.</p></div><button className="icon-button modal-close" onClick={() => setSettingsOpen(false)} title="Fechar"><X size={17} /></button></div>
            <div className="settings-block"><div className="setting-label"><span>Diretório de destino</span><small>Downloads padrão</small></div><code className="path-code">{engine?.downloadDir ?? "Carregando…"}</code><button className="folder-button full-width" onClick={() => void openFolder()}><FolderOpen size={15} /> Abrir pasta</button></div>
            <div className="settings-block"><div className="setting-label"><span>Perfil de desempenho</span><small>Turbo — recomendado para arquivos grandes</small></div><div className="tuning-grid"><div><span>Downloads simultâneos</span><strong>5</strong></div><div><span>Conexões por servidor</span><strong>16</strong></div><div><span>Divisão por arquivo</span><strong>16 partes</strong></div><div><span>Tamanho mínimo por parte</span><strong>1 MB</strong></div></div><p className="settings-note"><ShieldCheck size={14} /> Os limites são respeitados pelo servidor de origem. Aumentar números nem sempre aumenta a velocidade.</p></div>
            <div className="modal-footer"><span><HardDrive size={14} /> Retomada de downloads ativada</span><button className="primary-button" onClick={() => setSettingsOpen(false)}>Entendi <Check size={15} /></button></div>
          </section>
        </div>
      )}

      {toast && <div className="toast"><CheckCircle2 size={17} /><span>{toast}</span><button onClick={() => setToast("")} aria-label="Fechar notificação"><X size={14} /></button></div>}
    </div>
  );
}

function LayersIcon() {
  return <HardDrive size={18} />;
}

export default App;
