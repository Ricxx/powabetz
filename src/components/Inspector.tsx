import { useEffect, useState } from "react";
import { errMsg } from "../toast";
import { api } from "../api";
import type {
  FixtureInput,
  InspectFixture,
  PlayerInspect,
  TeamStatsView,
} from "../types";

function StatRow({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="flex justify-between text-xs py-0.5">
      <span className="text-slate-400">{label}</span>
      <span className="font-mono text-slate-100">{value}</span>
    </div>
  );
}

function TeamStats({ s }: { s: TeamStatsView }) {
  return (
    <div className="grid grid-cols-2 gap-x-4 mt-2 px-1">
      <StatRow label="played" value={s.played} />
      <StatRow label="ppg" value={s.ppg.toFixed(2)} />
      <StatRow label="goals for/g" value={s.gf_avg.toFixed(2)} />
      <StatRow label="goals ag/g" value={s.ga_avg.toFixed(2)} />
      <StatRow label="1st-half share" value={`${Math.round(s.first_half_share * 100)}%`} />
      <StatRow label="fail-to-score" value={`${Math.round(s.fts_rate * 100)}%`} />
    </div>
  );
}

function PlayerDetail({ p, onBack }: { p: PlayerInspect; onBack: () => void }) {
  return (
    <div className="space-y-3">
      <button className="btn btn-ghost text-sm py-2 px-3" onClick={onBack}>
        ← teams
      </button>
      <div>
        <div className="font-bold">{p.name}</div>
        <div className="text-xs text-slate-400">
          {p.position} · {p.apps} apps · {p.minutes} min
        </div>
      </div>
      <div className="card">
        <div className="text-xs font-semibold text-slate-400 mb-1">Season totals</div>
        <div className="grid grid-cols-2 gap-x-4">
          <StatRow label="goals" value={p.goals} />
          <StatRow label="shots" value={p.shots} />
          <StatRow label="on target" value={p.sot} />
          <StatRow label="tackles" value={p.tackles} />
          <StatRow label="fouls made" value={p.fouls_committed} />
          <StatRow label="fouls won" value={p.fouls_drawn} />
          <StatRow label="cards" value={p.cards} />
          <StatRow label="passes" value={p.passes} />
        </div>
      </div>
      <div className="card">
        <div className="text-xs font-semibold text-slate-400 mb-1">Per 90 (engine inputs)</div>
        <div className="grid grid-cols-2 gap-x-4">
          <StatRow label="goals/90" value={p.per90.goals.toFixed(2)} />
          <StatRow label="sot/90" value={p.per90.sot.toFixed(2)} />
          <StatRow label="shots/90" value={p.per90.shots.toFixed(2)} />
          <StatRow label="tackles/90" value={p.per90.tackles.toFixed(2)} />
          <StatRow label="fouls/90" value={p.per90.fouls.toFixed(2)} />
          <StatRow label="cards/90" value={p.per90.cards.toFixed(2)} />
          <StatRow label="passes/90" value={p.per90.passes.toFixed(0)} />
        </div>
      </div>
    </div>
  );
}

export default function Inspector({
  fixtures,
  onClose,
}: {
  fixtures: FixtureInput[];
  onClose: () => void;
}) {
  const [data, setData] = useState<InspectFixture[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [shown, setShown] = useState(false);
  const [openTeam, setOpenTeam] = useState<string | null>(null);
  const [player, setPlayer] = useState<PlayerInspect | null>(null);
  const [loadingPlayer, setLoadingPlayer] = useState(false);
  const [teamLoading, setTeamLoading] = useState<Set<string>>(new Set());

  useEffect(() => {
    setShown(true);
    api.inspectFixtures(fixtures).then(setData).catch((e) => setErr(errMsg(e)));
  }, []);

  async function openPlayer(playerId: number, leagueId: number, season: number) {
    setLoadingPlayer(true);
    setPlayer(null);
    try {
      const p = await api.inspectPlayer(playerId, leagueId, season);
      if (p) setPlayer(p);
      else setErr("No season stats found for this player in this league/season.");
    } catch (e) {
      setErr(errMsg(e));
    } finally {
      setLoadingPlayer(false);
    }
  }

  async function loadTeamStats(fx: InspectFixture, teamId: number, key: string) {
    setTeamLoading((s) => new Set(s).add(key));
    try {
      const stats = await api.inspectTeamStats(teamId, fx.league_id, fx.season);
      setData(
        (prev) =>
          prev?.map((f) =>
            f.fixture_id !== fx.fixture_id
              ? f
              : { ...f, teams: f.teams.map((tt) => (tt.team_id !== teamId ? tt : { ...tt, stats })) }
          ) ?? prev
      );
    } catch (e) {
      setErr(errMsg(e));
    } finally {
      setTeamLoading((s) => {
        const n = new Set(s);
        n.delete(key);
        return n;
      });
    }
  }

  return (
    <div className="fixed inset-0 z-40">
      <div className="absolute inset-0 bg-black/60" onClick={onClose} />
      <div
        className={`absolute top-0 right-0 h-full w-[88%] max-w-md bg-ink border-l border-edge shadow-2xl transition-transform duration-200 ${
          shown ? "translate-x-0" : "translate-x-full"
        } flex flex-col`}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-edge">
          <div className="font-bold">Data viewer</div>
          <button className="btn btn-ghost text-sm py-2" onClick={onClose}>
            Close
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-4 space-y-4">
          {err && (
            <div className="text-xs text-warn">
              {err}
              <button className="ml-2 underline" onClick={() => setErr(null)}>
                dismiss
              </button>
            </div>
          )}

          {loadingPlayer && <div className="text-sm text-slate-400">Loading player…</div>}

          {player ? (
            <PlayerDetail p={player} onBack={() => setPlayer(null)} />
          ) : (
            <>
              {!data && !err && <div className="text-sm text-slate-400">Reading cached data…</div>}
              {data?.length === 0 && (
                <div className="text-sm text-slate-400">No fixtures selected.</div>
              )}
              {data?.map((fx) => (
                <div key={fx.fixture_id} className="space-y-2">
                  <div className="text-xs font-semibold text-slate-400">{fx.fixture_label}</div>
                  {fx.teams.map((t) => {
                    const key = `${fx.fixture_id}-${t.team_id}`;
                    const open = openTeam === key;
                    return (
                      <div key={key} className="card">
                        <button
                          className="w-full flex items-center justify-between"
                          onClick={() => setOpenTeam(open ? null : key)}
                        >
                          <span className="font-semibold">{t.team_name}</span>
                          <span className="text-xs text-slate-400">
                            {t.players.length} players {open ? "▴" : "▾"}
                          </span>
                        </button>

                        {open && (
                          <div className="mt-2">
                            {t.stats ? (
                              <TeamStats s={t.stats} />
                            ) : (
                              <button
                                className="btn btn-ghost text-xs w-full"
                                disabled={teamLoading.has(key)}
                                onClick={() => loadTeamStats(fx, t.team_id, key)}
                              >
                                {teamLoading.has(key) ? "Loading…" : "Load team stats"}
                              </button>
                            )}
                            {!t.loaded && (
                              <div className="text-[11px] text-slate-500 px-1 mt-1">
                                Squad not cached yet — run a Build (or open the Data board) for this match first.
                              </div>
                            )}
                            <div className="flex flex-wrap gap-1.5 mt-3">
                              {t.players.map((p) => {
                                const out =
                                  p.availability === "injured" || p.availability === "suspended";
                                return (
                                  <button
                                    key={p.player_id}
                                    className={`chip text-xs ${out ? "chip-dim" : ""}`}
                                    onClick={() => openPlayer(p.player_id, fx.league_id, fx.season)}
                                  >
                                    {p.name}
                                    {out && <span className="ml-1 text-bad">⚑</span>}
                                  </button>
                                );
                              })}
                            </div>
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              ))}
              <p className="text-[10px] text-slate-500">
                Reads only cached data — browsing here costs 0 requests. Tap a player for the exact
                per-90 numbers the engine uses.
              </p>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
