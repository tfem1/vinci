import { Film, FolderOpen, RefreshCw, Search, Star } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { ClipCard } from "../components/ClipCard";
import { Select } from "../components/ui";
import { useEngine } from "../hooks/engine";
import { api, errorText } from "../lib/api";
import { formatBytes, formatDuration } from "../lib/format";
import type { Clip, LibraryStats, SortBy } from "../lib/types";

export function Library({ onOpen }: { onOpen: (c: Clip) => void }) {
  const { libraryVersion, toast } = useEngine();
  const [clips, setClips] = useState<Clip[] | null>(null);
  const [stats, setStats] = useState<LibraryStats | null>(null);
  const [search, setSearch] = useState("");
  const [game, setGame] = useState<string>("");
  const [fav, setFav] = useState(false);
  const [sort, setSort] = useState<SortBy>("newest");
  const [scanning, setScanning] = useState(false);

  const query = useMemo(() => ({ search: search || null, game: game || null, favorites_only: fav, sort }), [search, game, fav, sort]);

  useEffect(() => {
    const t = setTimeout(() => {
      api.listClips(query).then(setClips).catch((e) => toast({ kind: "error", title: "Library", detail: errorText(e) }));
    }, 120);
    return () => clearTimeout(t);
  }, [query, libraryVersion, toast]);

  useEffect(() => {
    api.libraryStats().then(setStats).catch(() => {});
  }, [libraryVersion]);

  const scan = async () => {
    setScanning(true);
    try {
      const r = await api.scanLibrary();
      toast({ kind: "info", title: "Library synced", detail: `${r.added} added · ${r.removed} removed · ${r.updated} updated` });
      setClips(await api.listClips(query));
      setStats(await api.libraryStats());
    } catch (e) {
      toast({ kind: "error", title: "Sync failed", detail: errorText(e) });
    }
    setScanning(false);
  };

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Library</h1>
          <p className="num">
            {stats ? `${stats.count} clips · ${formatDuration(stats.total_duration)} · ${formatBytes(stats.total_bytes)} on disk` : "Loading…"}
          </p>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn" onClick={scan} disabled={scanning} title="Re-read the clips folder">
            <RefreshCw size={14} className={scanning ? "spin" : ""} /> Sync folder
          </button>
          <button className="btn" onClick={() => api.openClipsFolder().catch((e) => toast({ kind: "error", title: "Could not open folder", detail: errorText(e) }))}>
            <FolderOpen size={14} /> Open folder
          </button>
        </div>
      </div>

      <div className="toolbar">
        <div className="search">
          <Search size={14} />
          <input className="input" placeholder="Search clips and games" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Search" />
        </div>
        <button className={`chip ${fav ? "on" : ""}`} onClick={() => setFav((f) => !f)}>
          <Star size={12} fill={fav ? "currentColor" : "none"} /> Favorites
        </button>
        <div className="grow" />
        <Select<string> label="Game" value={game} onChange={setGame} width={190} options={[{ value: "", label: "All games" }, ...(stats?.games ?? []).map((g) => ({ value: g.game, label: `${g.game} (${g.count})` }))]} />
        <Select<SortBy>
          label="Sort"
          value={sort}
          onChange={setSort}
          width={150}
          options={[
            { value: "newest", label: "Newest first" },
            { value: "oldest", label: "Oldest first" },
            { value: "largest", label: "Largest" },
            { value: "smallest", label: "Smallest" },
            { value: "longest", label: "Longest" },
            { value: "shortest", label: "Shortest" },
            { value: "name", label: "Name" },
          ]}
        />
      </div>

      {clips === null ? null : clips.length === 0 ? (
        <div className="card empty">
          <Film size={28} />
          <b>{search || game || fav ? "No clip matches" : "Your library is empty"}</b>
          <span>{search || game || fav ? "Try another search or filter." : "Saved clips appear here and in your clips folder — one file each, never duplicated."}</span>
        </div>
      ) : (
        <div className="clip-grid">
          {clips.map((c) => (
            <ClipCard key={c.id} clip={c} onOpen={onOpen} />
          ))}
        </div>
      )}
    </div>
  );
}
