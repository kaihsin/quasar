import { useEffect, useRef, useState } from "react";

import { fetchPeople, fetchPersonWorkItems } from "../api";
import type { PersonWorkItems } from "../types";
import WorkItemCard from "./WorkItemCard";

export default function PeoplePage({ onOpenItem }: { onOpenItem: (id: string) => void }) {
  const [people, setPeople] = useState<string[]>([]);
  const [selected, setSelected] = useState("");
  const [data, setData] = useState<PersonWorkItems | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const controllerRef = useRef<AbortController | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    fetchPeople(controller.signal)
      .then((users) => {
        if (!controller.signal.aborted) setPeople(users);
      })
      .catch(() => {
        /* best-effort; leave empty */
      });
    return () => controller.abort();
  }, []);

  useEffect(() => () => controllerRef.current?.abort(), []);

  function selectPerson(user: string) {
    setSelected(user);
    setData(null);
    setError(null);
    controllerRef.current?.abort();
    if (!user) return;
    const controller = new AbortController();
    controllerRef.current = controller;
    setIsLoading(true);
    fetchPersonWorkItems(user, controller.signal)
      .then((result) => {
        if (!controller.signal.aborted) setData(result);
      })
      .catch((err: unknown) => {
        if (!controller.signal.aborted) {
          setError(err instanceof Error ? err.message : "Failed to load");
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) setIsLoading(false);
      });
  }

  return (
    <section aria-label="People" className="people-page">
      <div className="filter-field">
        <label htmlFor="person-select">Person</label>
        <select
          id="person-select"
          onChange={(event) => selectPerson(event.target.value)}
          value={selected}
        >
          <option value="">Select a person…</option>
          {people.map((user) => (
            <option key={user} value={user}>
              {user}
            </option>
          ))}
        </select>
      </div>

      {isLoading ? <p className="empty-state">Loading…</p> : null}
      {error ? <p className="error-banner">Failed to load: {error}</p> : null}

      {data ? (
        <div className="people-sections">
          <section aria-label="Created by" className="people-section">
            <h3>Created by ({data.created_by.length})</h3>
            {data.created_by.length ? (
              data.created_by.map((item) => (
                <WorkItemCard item={item} key={item.id} onOpen={() => onOpenItem(item.id)} />
              ))
            ) : (
              <p className="board-column-empty">Nothing here</p>
            )}
          </section>
          <section aria-label="Mentioned" className="people-section">
            <h3>Mentioned ({data.mentioned.length})</h3>
            {data.account_id === null ? (
              <p className="board-column-empty">
                Couldn't resolve this person's account; mentions unavailable.
              </p>
            ) : data.mentioned.length ? (
              data.mentioned.map((item) => (
                <WorkItemCard item={item} key={item.id} onOpen={() => onOpenItem(item.id)} />
              ))
            ) : (
              <p className="board-column-empty">Nothing here</p>
            )}
          </section>
        </div>
      ) : null}
    </section>
  );
}
