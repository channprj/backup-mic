import { HardDriveIcon, ShieldCheckIcon } from "lucide-react";
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
import type { AppSnapshot } from "./contracts";

interface SetupFlowProps {
  snapshot: AppSnapshot;
  busy: boolean;
  onChooseDestination: () => Promise<unknown>;
}

export function SetupFlow({ snapshot, busy, onChooseDestination }: SetupFlowProps) {
  if (snapshot.setup_state === "ready") {
    return (
      <Card>
        <CardHeader>
          <ShieldCheckIcon className="setup-icon" aria-hidden="true" />
          <CardTitle>백업 준비가 끝났습니다</CardTitle>
          <CardDescription>
            DJI Mic Mini 2S 기본 규칙이 등록되어 있습니다. 다른 녹음기는 Settings에서
            추가할 수 있습니다.
          </CardDescription>
        </CardHeader>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <HardDriveIcon className="setup-icon" aria-hidden="true" />
        <CardTitle>백업 폴더를 확인해 주세요</CardTitle>
        <CardDescription>
          녹음은 선택한 폴더 안에서 녹음기별 규칙과 날짜에 따라 정리됩니다.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <p className="setup-note">
          DJI Mic Mini 2S 기본 규칙은 이미 준비되어 있으며 폴더 경로는 앱 밖으로 전송되지
          않습니다.
        </p>
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
