import { useEffect, useMemo, useState } from "react";
import { HardDriveIcon, Link2Icon, RadioTowerIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { AppSnapshot, PairingAssignment, Transmitter } from "./contracts";
import { formatBytes } from "./format";

interface SetupFlowProps {
  snapshot: AppSnapshot;
  busy: boolean;
  onChooseDestination: () => Promise<unknown>;
  onPair: (assignments: PairingAssignment[]) => Promise<unknown>;
}

function defaultAssignments(snapshot: AppSnapshot) {
  return Object.fromEntries(
    snapshot.pairing_candidates.map(({ candidate_id }, index) => [
      candidate_id,
      index === 0 ? "TX01" : "TX02",
    ]),
  ) as Record<string, Transmitter>;
}

export function SetupFlow({
  snapshot,
  busy,
  onChooseDestination,
  onPair,
}: SetupFlowProps) {
  const candidateKey = snapshot.pairing_candidates
    .map(({ candidate_id }) => candidate_id)
    .join(":");
  const [assignments, setAssignments] = useState<Record<string, Transmitter>>(() =>
    defaultAssignments(snapshot),
  );

  useEffect(() => {
    setAssignments(defaultAssignments(snapshot));
    // Candidate IDs change on reconnect, so a changed key deliberately resets labels.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [candidateKey]);

  const pairingPayload = useMemo(
    () =>
      snapshot.pairing_candidates.flatMap(({ candidate_id }) => {
        const transmitter = assignments[candidate_id];
        return transmitter ? [{ candidate_id, transmitter }] : [];
      }),
    [assignments, snapshot.pairing_candidates],
  );

  if (snapshot.setup_state === "needs_destination") {
    return (
      <Card>
        <CardHeader>
          <HardDriveIcon className="setup-icon" aria-hidden="true" />
          <CardTitle>백업 폴더를 확인해 주세요</CardTitle>
          <CardDescription>
            녹음은 Documents의 DJI-Mic-Mini-2S 폴더에 날짜별로 정리됩니다.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p className="setup-note">폴더 경로는 앱 밖으로 전송되지 않습니다.</p>
        </CardContent>
        <CardFooter>
          <Button disabled={busy} onClick={() => void onChooseDestination()}>
            {busy ? <Spinner data-icon="inline-start" /> : <HardDriveIcon data-icon="inline-start" />}
            백업 폴더 선택
          </Button>
        </CardFooter>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <RadioTowerIcon className="setup-icon" aria-hidden="true" />
        <CardTitle>송신기를 한 번만 연결해 주세요</CardTitle>
        <CardDescription>
          케이스를 USB-C로 연결한 뒤 각 저장 장치를 TX01과 TX02에 지정합니다.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {snapshot.pairing_candidates.length === 0 ? (
          <div className="waiting-device" role="status">
            <span className="waiting-pulse" aria-hidden="true" />
            DJI Mic 케이스 연결을 기다리는 중
          </div>
        ) : snapshot.pairing_candidates.length < 2 ? (
          <div className="waiting-device" role="status">
            <span className="waiting-pulse" aria-hidden="true" />
            두 번째 송신기 저장 장치를 기다리는 중
          </div>
        ) : (
          <div className="candidate-list">
            {snapshot.pairing_candidates.map((candidate) => (
              <div className="candidate-row" key={candidate.candidate_id}>
                <div className="candidate-copy">
                  <strong>{candidate.display_name}</strong>
                  <span>{formatBytes(candidate.capacity_bytes)}</span>
                </div>
                <ToggleGroup
                  type="single"
                  variant="outline"
                  size="sm"
                  value={assignments[candidate.candidate_id] ?? ""}
                  aria-label={`${candidate.display_name} 송신기 지정`}
                  onValueChange={(next: Transmitter) => {
                    if (!next) return;
                    setAssignments((current) => {
                      const previous = current[candidate.candidate_id];
                      const occupied = Object.entries(current).find(
                        ([candidateId, transmitter]) =>
                          candidateId !== candidate.candidate_id && transmitter === next,
                      );
                      return {
                        ...current,
                        [candidate.candidate_id]: next,
                        ...(occupied && previous ? { [occupied[0]]: previous } : {}),
                      };
                    });
                  }}
                >
                  <ToggleGroupItem value="TX01" aria-label="TX01로 지정">
                    TX01
                  </ToggleGroupItem>
                  <ToggleGroupItem value="TX02" aria-label="TX02로 지정">
                    TX02
                  </ToggleGroupItem>
                </ToggleGroup>
              </div>
            ))}
          </div>
        )}
      </CardContent>
      <CardFooter>
        <Button
          disabled={
            busy ||
            snapshot.pairing_candidates.length !== 2 ||
            pairingPayload.length !== 2
          }
          onClick={() => void onPair(pairingPayload)}
        >
          {busy ? <Spinner data-icon="inline-start" /> : <Link2Icon data-icon="inline-start" />}
          이 송신기로 연결
        </Button>
      </CardFooter>
    </Card>
  );
}
