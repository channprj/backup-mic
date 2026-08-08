# DJI Mic Mini 2S Automatic Backup Implementation Plan

> **Status:** Superseded by `2026-08-09-dji-mic-mini-tauri-rust-backup.md`. This Swift-native plan is retained as design history and must not be executed.

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan sequentially in the main thread. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a login-launched macOS menu bar app that detects the user's two paired DJI Mic Mini transmitter volumes, automatically creates SHA-256-verified local backups, reports the outcome clearly, and deletes a transmitter's complete verified WAV snapshot only after explicit user confirmation.

**Architecture:** A single Swift 6 application uses Disk Arbitration for volume lifecycle events, Foundation and CryptoKit for stable scanning and verified streaming copies, and an actor-isolated SQLite ledger for device pairing, deduplication, recovery, and deletion eligibility. A SwiftUI `MenuBarExtra` provides persistent status while small AppKit panels handle first-run pairing, destination approval, and destructive confirmation. The app is built with Swift Package Manager and assembled into a signed `.app` bundle so the current Command Line Tools installation is sufficient; no privileged helper, daemon, or full Xcode project is required.

**Tech Stack:** Swift 6.3.3, Swift Package Manager, SwiftUI/AppKit, DiskArbitration, CryptoKit SHA-256, Foundation, SQLite 3.51, UserNotifications, ServiceManagement `SMAppService`, XCTest, shell packaging with `codesign` and `plutil`.

## Global Constraints

- Target macOS 13 or newer; `MenuBarExtra` and `SMAppService` establish the deployment floor.
- Build from the installed Command Line Tools with `swift build` and `swift test`; do not require a generated Xcode project.
- Use the bundle identifier `com.channprj.DJIMicBackup` and the product name `DJI Mic Backup`.
- Use `/Users/channprj/Documents/DJI-Mic-Mini-2S` as the initial destination selected and confirmed during first-run setup.
- Preserve at least 10 GiB of destination free space after the bytes required by a run.
- Treat volume labels and mount paths such as `/Volumes/DJI-MIC-1` as mutable display data, never as device authority.
- Trust a transmitter only after pairing its volume UUID with removable, external, writable USB media properties and media name `Mic Tx` or `Wireless Mic Tx Media`.
- Discover only regular, non-symlink `.wav` files in non-hidden source directories; never recursively delete directories, hidden files, trash folders, or unknown types.
- Require two metadata observations two seconds apart before a WAV file is stable enough to copy.
- Copy into a hidden temporary file in the final directory, synchronize it, compare source and destination byte counts and SHA-256 values, then rename it to the final name on the same destination volume.
- Store user-visible backups as `YYYY/YYYY-MM-DD/TX01|TX02/<original-filename>.wav`.
- Never overwrite a different existing file; identical content is deduplicated and different content receives an eight-character SHA-256 suffix.
- Never delete automatically. Enable deletion per transmitter only when every currently discoverable WAV file belongs to one complete verified snapshot.
- Re-scan and re-hash every source and destination file before unlinking the first source file. If preflight differs, delete nothing.
- Multi-file unlink is not filesystem-atomic. If an unlink fails after earlier files were removed, stop, preserve the remaining files, record the exact partial outcome, and report it prominently; every removed file remains backed up and verified.
- Existing recordings on the connected devices are production data, not test fixtures. Hardware deletion tests must use newly created disposable recordings.
- Keep audio, filenames, paths, hashes, and operational metadata local; use unified logging with dynamic paths and names explicitly marked private.
- Do not add analytics, networking, cloud synchronization, audio playback, transcription, automatic ejection, or a general settings window.

---

## Source Documents and Verified Environment

- Product design: `docs/superpowers/specs/2026-08-09-dji-mic-mini-auto-backup-design.md`
- Current project directory is empty apart from planning documents and is not yet a Git repository.
- Local compiler: Swift 6.3.3 targeting arm64 macOS.
- Local SDK: macOS 26.5 from Command Line Tools.
- Local SQLite: 3.51.0, importable as `SQLite3` and linkable with `-lsqlite3`.
- Available packaging tools: `codesign`, `plutil`, `hdiutil`, and `pkgbuild`.
- Current hardware presents two FAT32 physical USB volumes with stable distinct UUIDs and mutable labels `DJI-MIC-1` and `DJI-MIC-2`. The plan-time inventory contains nine files matching the observed `TX01_MIC009_20260809_021728_edit.wav` shape and two files matching the observed `TX02_MIC002_20260809_021844_edit.wav` shape.

## File Structure

```text
.
├── Package.swift
├── .gitignore
├── README.md
├── Packaging
│   └── Info.plist
├── Sources
│   ├── DJIMicBackupCore
│   │   ├── Backup
│   │   │   ├── BackupCoordinator.swift
│   │   │   ├── DestinationPlanner.swift
│   │   │   ├── FileMetadata.swift
│   │   │   ├── SafeFileCopier.swift
│   │   │   └── SHA256FileHasher.swift
│   │   ├── Deletion
│   │   │   └── DeletionCoordinator.swift
│   │   ├── Device
│   │   │   ├── DeviceMatcher.swift
│   │   │   ├── DeviceModels.swift
│   │   │   ├── DeviceRegistry.swift
│   │   │   └── DiskArbitrationMonitor.swift
│   │   ├── Discovery
│   │   │   ├── DJIRecordingName.swift
│   │   │   ├── RecordingCandidate.swift
│   │   │   └── RecordingScanner.swift
│   │   ├── Domain
│   │   │   ├── BackupModels.swift
│   │   │   └── BackupStatus.swift
│   │   ├── Persistence
│   │   │   ├── BackupLedger.swift
│   │   │   ├── LedgerModels.swift
│   │   │   └── SQLiteConnection.swift
│   │   └── Support
│   │       ├── AppPaths.swift
│   │       ├── Clock.swift
│   │       └── PrivateLogger.swift
│   └── DJIMicBackupApp
│       ├── AppModel.swift
│       ├── DJIMicBackupApp.swift
│       ├── DeletionConfirmation.swift
│       ├── DestinationAccess.swift
│       ├── LoginItemClient.swift
│       ├── MenuBarContent.swift
│       ├── NotificationClient.swift
│       ├── SetupWindowController.swift
│       └── SetupView.swift
├── Tests
│   ├── DJIMicBackupAppTests
│   │   └── AppModelTests.swift
│   └── DJIMicBackupCoreTests
│       ├── BackupStatusTests.swift
│       ├── BackupCoordinatorTests.swift
│       ├── BackupLedgerTests.swift
│       ├── DJIRecordingNameTests.swift
│       ├── DeletionCoordinatorTests.swift
│       ├── DestinationPlannerTests.swift
│       ├── DeviceMatcherTests.swift
│       ├── RecordingScannerTests.swift
│       ├── SafeFileCopierTests.swift
│       └── TestSupport.swift
└── scripts
    ├── build-app.sh
    ├── install-app.sh
    └── smoke-test.sh
```

`DJIMicBackupCore` contains all filesystem policy and is testable without SwiftUI. `DJIMicBackupApp` is a thin main-actor adapter over the core. `Packaging/Info.plist` is copied into the bundle by the build script; the SwiftPM executable itself remains the only binary.

---

### Task 1: Establish the package, domain contract, and recoverable build skeleton

**Files:**
- Create: `.gitignore`
- Create: `Package.swift`
- Create: `Sources/DJIMicBackupCore/Domain/BackupStatus.swift`
- Create: `Sources/DJIMicBackupCore/Domain/BackupModels.swift`
- Create: `Sources/DJIMicBackupCore/Support/Clock.swift`
- Create: `Sources/DJIMicBackupApp/DJIMicBackupApp.swift`
- Create: `Tests/DJIMicBackupCoreTests/BackupStatusTests.swift`

**Interfaces:**
- Produces: `TransmitterLabel`, `BackupPhase`, `PerDeviceOutcome`, `BackupRunOutcome`, `Clock`, and the `DJIMicBackup` executable target.
- Consumes: Approved constraints from the design document only.

- [ ] **Step 1: Initialize version control without touching external volumes**

Run from the project root:

```bash
git init
git branch -M main
```

Expected: `git status --short --branch` reports an empty `main` branch and only the planning documents as untracked.

- [ ] **Step 2: Create the Swift package manifest**

Use this target graph so the app remains thin and both targets can be tested:

```swift
// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "DJIMicBackup",
    platforms: [.macOS(.v13)],
    products: [
        .library(name: "DJIMicBackupCore", targets: ["DJIMicBackupCore"]),
        .executable(name: "DJIMicBackup", targets: ["DJIMicBackupApp"]),
    ],
    targets: [
        .target(
            name: "DJIMicBackupCore",
            linkerSettings: [.linkedLibrary("sqlite3")]
        ),
        .executableTarget(
            name: "DJIMicBackupApp",
            dependencies: ["DJIMicBackupCore"]
        ),
        .testTarget(
            name: "DJIMicBackupCoreTests",
            dependencies: ["DJIMicBackupCore"]
        ),
        .testTarget(
            name: "DJIMicBackupAppTests",
            dependencies: ["DJIMicBackupApp", "DJIMicBackupCore"]
        ),
    ]
)
```

Add `.build/`, `build/`, `*.app`, `.DS_Store`, and `*.sqlite-*` to `.gitignore`. Do not ignore the design or implementation plan.

- [ ] **Step 3: Write the failing domain-state tests**

Create explicit tests for the status labels and destructive-action gate:

```swift
import XCTest
@testable import DJIMicBackupCore

final class BackupStatusTests: XCTestCase {
    func testDeletionIsEnabledOnlyForCompleteVerifiedSnapshot() {
        XCTAssertFalse(BackupPhase.idle.canRequestDeletion)
        XCTAssertFalse(BackupPhase.backingUp.canRequestDeletion)
        XCTAssertFalse(BackupPhase.partialFailure.canRequestDeletion)
        XCTAssertTrue(BackupPhase.verifiedDeletionPending.canRequestDeletion)
    }

    func testTransmitterLabelsRemainStable() {
        XCTAssertEqual(TransmitterLabel.tx01.rawValue, "TX01")
        XCTAssertEqual(TransmitterLabel.tx02.rawValue, "TX02")
    }
}
```

- [ ] **Step 4: Run the test to prove the domain is absent**

Run:

```bash
swift test --filter BackupStatusTests
```

Expected: compilation fails because `BackupPhase` and `TransmitterLabel` do not exist.

- [ ] **Step 5: Implement the stable domain types**

Define the state vocabulary exactly once:

```swift
public enum TransmitterLabel: String, Codable, CaseIterable, Sendable {
    case tx01 = "TX01"
    case tx02 = "TX02"
}

public enum BackupPhase: String, Codable, Sendable {
    case idle
    case deviceDetected
    case backingUp
    case verifiedDeletionPending
    case partialFailure
    case error

    public var canRequestDeletion: Bool {
        self == .verifiedDeletionPending
    }
}

public struct PerDeviceOutcome: Equatable, Sendable {
    public let label: TransmitterLabel
    public let phase: BackupPhase
    public let discoveredCount: Int
    public let verifiedCount: Int
    public let verifiedBytes: Int64
    public let failureMessages: [String]
}

public struct BackupRunOutcome: Equatable, Sendable {
    public let startedAt: Date
    public let finishedAt: Date
    public let devices: [PerDeviceOutcome]
}
```

Define `Clock` as a `Sendable` value with a `now: @Sendable () -> Date` closure and provide `.continuous` for production. This gives tests deterministic timestamps without global overrides.

- [ ] **Step 6: Add a compile-only app entry point and run the complete suite**

The first executable should prove SwiftUI linkage while containing no device behavior:

```swift
import SwiftUI

@main
struct DJIMicBackupApp: App {
    var body: some Scene {
        MenuBarExtra("DJI Mic Backup", systemImage: "externaldrive") {
            Text("설정 준비 중")
        }
    }
}
```

Run:

```bash
swift test
swift build --product DJIMicBackup
```

Expected: all tests pass and the executable links without a full Xcode installation.

- [ ] **Step 7: Commit the independently buildable skeleton**

```bash
git add .gitignore Package.swift Sources Tests docs/superpowers
git commit -m "feat: scaffold DJI mic backup app"
```

---

### Task 2: Detect, identify, and pair the two physical transmitter volumes

**Files:**
- Create: `Sources/DJIMicBackupCore/Device/DeviceModels.swift`
- Create: `Sources/DJIMicBackupCore/Device/DeviceMatcher.swift`
- Create: `Sources/DJIMicBackupCore/Device/DeviceRegistry.swift`
- Create: `Sources/DJIMicBackupCore/Device/DiskArbitrationMonitor.swift`
- Create: `Tests/DJIMicBackupCoreTests/DeviceMatcherTests.swift`
- Create: `Tests/DJIMicBackupCoreTests/TestSupport.swift`

**Interfaces:**
- Consumes: `TransmitterLabel`, `Clock`.
- Produces: `MountedVolume`, `PairedDevice`, `TrustedMountedDevice`, `DeviceEvent`, `VolumeMonitoring`, `DeviceMatching`, and `DeviceRegistryProtocol`.

- [ ] **Step 1: Define immutable volume and pairing records**

Use these exact fields so volume labels remain display-only:

```swift
public struct MountedVolume: Hashable, Sendable {
    public let volumeUUID: UUID
    public let volumeName: String
    public let mediaName: String
    public let deviceProtocol: String
    public let capacityBytes: Int64
    public let isInternal: Bool
    public let isRemovable: Bool
    public let isWritable: Bool
    public let mountURL: URL
}

public struct PairedDevice: Hashable, Codable, Sendable {
    public let volumeUUID: UUID
    public let label: TransmitterLabel
    public let mediaName: String
    public let capacityBytes: Int64
}

public struct TrustedMountedDevice: Hashable, Sendable {
    public let pairing: PairedDevice
    public let volume: MountedVolume
}

public enum DeviceEvent: Sendable {
    case appeared(MountedVolume)
    case disappeared(volumeUUID: UUID)
}
```

- [ ] **Step 2: Write matcher tests that reject every name-only shortcut**

Test at least these cases with fixture URLs under a temporary directory:

```swift
func testExactPairingAcceptsMutableFriendlyVolumeLabel() throws {
    let pairing = Fixtures.pairing(uuid: Fixtures.tx01UUID, label: .tx01)
    let mounted = Fixtures.mounted(
        uuid: Fixtures.tx01UUID,
        volumeName: "RENAMED-LATER",
        mediaName: "Mic Tx",
        deviceProtocol: "USB",
        capacityBytes: pairing.capacityBytes,
        isInternal: false,
        isRemovable: true,
        isWritable: true
    )
    XCTAssertNotNil(DeviceMatcher().match(mounted, against: [pairing]))
}

func testNameOnlyNoNameVolumeIsRejected() throws {
    let unknown = Fixtures.mounted(
        uuid: UUID(),
        volumeName: "NO NAME",
        mediaName: "Mic Tx",
        deviceProtocol: "USB",
        capacityBytes: 15_636_365_312,
        isInternal: false,
        isRemovable: true,
        isWritable: true
    )
    XCTAssertNil(DeviceMatcher().match(unknown, against: Fixtures.pairings))
}
```

Add separate assertions rejecting UUID matches whose media is internal, not removable, read-only, non-USB, has an unexpected media name, or differs in nominal capacity by more than one percent.

- [ ] **Step 3: Run the matcher tests and verify they fail**

```bash
swift test --filter DeviceMatcherTests
```

Expected: compilation fails because the device models and matcher are not implemented.

- [ ] **Step 4: Implement fail-closed matching**

`DeviceMatcher.match(_:against:)` returns a `TrustedMountedDevice?` only when all conditions hold:

```swift
public protocol DeviceMatching: Sendable {
    func match(_ volume: MountedVolume, against pairings: [PairedDevice]) -> TrustedMountedDevice?
}

public struct DeviceMatcher: DeviceMatching {
    public init() {}

    public func match(
        _ volume: MountedVolume,
        against pairings: [PairedDevice]
    ) -> TrustedMountedDevice? {
        guard let pairing = pairings.first(where: { $0.volumeUUID == volume.volumeUUID }),
              volume.deviceProtocol.caseInsensitiveCompare("USB") == .orderedSame,
              volume.isInternal == false,
              volume.isRemovable,
              volume.isWritable,
              ["Mic Tx", "Wireless Mic Tx Media"].contains(volume.mediaName),
              abs(volume.capacityBytes - pairing.capacityBytes) <= pairing.capacityBytes / 100
        else { return nil }

        return TrustedMountedDevice(pairing: pairing, volume: volume)
    }
}
```

The matching function must not inspect `volumeName` or the mount-path string.

- [ ] **Step 5: Implement Disk Arbitration event delivery**

Create a `DASession`, register appeared, disappeared, and description-changed callbacks, and schedule it on a dedicated serial dispatch queue with `DASessionSetDispatchQueue`. Convert `DADiskCopyDescription` keys into `MountedVolume` using:

- `kDADiskDescriptionVolumeUUIDKey`
- `kDADiskDescriptionVolumeNameKey`
- `kDADiskDescriptionVolumePathKey`
- `kDADiskDescriptionMediaNameKey`
- `kDADiskDescriptionDeviceProtocolKey`
- `kDADiskDescriptionMediaSizeKey`
- `kDADiskDescriptionDeviceInternalKey`
- `kDADiskDescriptionMediaRemovableKey`
- `kDADiskDescriptionMediaWritableKey`

Use an `AsyncStream<DeviceEvent>` interface:

```swift
public protocol VolumeMonitoring: Sendable {
    func events() -> AsyncStream<DeviceEvent>
    func start() throws
    func stop()
}
```

Do not emit an appeared event until both the UUID and mounted volume URL exist. The description-changed callback handles the case where Disk Arbitration reports the disk before mount completion. Retain and release the callback context with one balanced `Unmanaged` lifetime owned by the monitor.

- [ ] **Step 6: Add a registry boundary without choosing persistence twice**

Define the async protocol now and provide an in-memory implementation for tests. Task 4 supplies SQLite persistence.

```swift
public protocol DeviceRegistryProtocol: Sendable {
    func allPairings() async throws -> [PairedDevice]
    func pairing(for volumeUUID: UUID) async throws -> PairedDevice?
    func save(_ pairing: PairedDevice) async throws
    func remove(volumeUUID: UUID) async throws
}
```

Pairing must enforce unique volume UUIDs and unique logical labels so the same physical transmitter cannot be both `TX01` and `TX02`.

- [ ] **Step 7: Run unit tests and a read-only hardware observation smoke check**

```bash
swift test --filter DeviceMatcherTests
swift test
```

Expected: tests pass. The implementation-time smoke helper may print detected device properties, but it must contain no copy or delete call and must not modify the connected FAT32 volumes.

- [ ] **Step 8: Commit device identity and monitoring**

```bash
git add Sources/DJIMicBackupCore/Device Tests/DJIMicBackupCoreTests
git commit -m "feat: detect and pair DJI transmitters"
```

---

### Task 3: Discover stable recordings and choose deterministic collision-safe destinations

**Files:**
- Create: `Sources/DJIMicBackupCore/Discovery/DJIRecordingName.swift`
- Create: `Sources/DJIMicBackupCore/Discovery/RecordingCandidate.swift`
- Create: `Sources/DJIMicBackupCore/Discovery/RecordingScanner.swift`
- Create: `Sources/DJIMicBackupCore/Backup/FileMetadata.swift`
- Create: `Sources/DJIMicBackupCore/Backup/DestinationPlanner.swift`
- Create: `Tests/DJIMicBackupCoreTests/DJIRecordingNameTests.swift`
- Create: `Tests/DJIMicBackupCoreTests/RecordingScannerTests.swift`
- Create: `Tests/DJIMicBackupCoreTests/DestinationPlannerTests.swift`

**Interfaces:**
- Consumes: `TrustedMountedDevice`, `TransmitterLabel`, `Clock`.
- Produces: `DJIRecordingName`, `RecordingCandidate`, `RecordingScanning`, `FileMetadata`, `DestinationPlan`, and `DestinationPlanning`.

- [ ] **Step 1: Lock the DJI filename parser contract with failing tests**

Use the anchored, case-insensitive contract `TXdd_MICdigits_yyyyMMdd_HHmmss` followed by an optional suffix and `.wav`:

```swift
func testParsesObservedEditedFilenameWithoutTimezoneConversion() throws {
    let parsed = try XCTUnwrap(
        DJIRecordingName.parse("TX01_MIC009_20260809_021728_edit.wav")
    )
    XCTAssertEqual(parsed.label, .tx01)
    XCTAssertEqual(parsed.sequence, 9)
    XCTAssertEqual(parsed.dateDirectory, "2026-08-09")
}

func testRejectsTraversalAndNonWAVNames() {
    XCTAssertNil(DJIRecordingName.parse("../TX01_MIC001_20260809_000000.wav"))
    XCTAssertNil(DJIRecordingName.parse("TX01_MIC001_20260809_000000.mp3"))
}
```

Do not convert the filename timestamp through UTC. DJI's encoded calendar date directly determines the destination date directory.

- [ ] **Step 2: Implement the parser and candidate values**

The parser pattern is:

```swift
private static let pattern =
    #"^TX(\d{2})_MIC(\d+)_(\d{4})(\d{2})(\d{2})_(\d{2})(\d{2})(\d{2})(?:_[^.]+)?\.wav$"#
```

Return `nil` for unsupported transmitter labels and impossible calendar components. Define candidates with source-relative paths rather than trusted absolute strings:

```swift
public struct RecordingCandidate: Hashable, Sendable {
    public let device: TrustedMountedDevice
    public let sourceURL: URL
    public let relativePath: String
    public let originalFilename: String
    public let sizeBytes: Int64
    public let modificationDate: Date
    public let parsedName: DJIRecordingName?
}
```

- [ ] **Step 3: Write scanner tests using real temporary directories**

Tests must prove:

1. A regular `.wav` nested under a visible directory appears after two equal snapshots.
2. `.Spotlight-V100`, `.Trashes`, `.fseventsd`, symlinks, directories, MP3 files, and files outside the volume root are ignored.
3. A file whose size or modification time changes between snapshots is deferred.
4. A `TX02_` filename on the device paired as `TX01` is rejected and surfaced as a scan issue.
5. An empty paired transmitter returns an empty successful result.

Use a test settling interval of 20 milliseconds while production passes two seconds.

- [ ] **Step 4: Implement safe enumeration and two-observation stability**

```swift
public protocol RecordingScanning: Sendable {
    func stableRecordings(on device: TrustedMountedDevice) async throws -> ScanResult
}

public struct ScanResult: Sendable {
    public let recordings: [RecordingCandidate]
    public let issues: [ScanIssue]
}

public enum ScanIssue: Equatable, Sendable {
    case transmitterLabelMismatch(relativePath: String)
    case metadataUnavailable(relativePath: String)
    case changedDuringSettling(relativePath: String)
    case escapedVolumeRoot(relativePath: String)
}
```

For each URL, request `.isRegularFileKey`, `.isSymbolicLinkKey`, `.fileSizeKey`, and `.contentModificationDateKey`. Standardize both root and candidate URLs and require every candidate path to have the root's path-components prefix. Re-enumerate after the settling delay and keep only the intersection whose relative path, size, and modification date are identical.

- [ ] **Step 5: Write destination-planning tests**

Cover:

```swift
func testObservedNameMapsToExpectedLayout() throws {
    let plan = try planner.plan(
        candidate: Fixtures.tx01Candidate(
            filename: "TX01_MIC009_20260809_021728_edit.wav"
        ),
        sourceDigest: Fixtures.digestA,
        destinationRoot: Fixtures.destinationRoot,
        existing: .none
    )
    XCTAssertEqual(
        plan.finalURL.path,
        Fixtures.destinationRoot
            .appending(path: "2026/2026-08-09/TX01/TX01_MIC009_20260809_021728_edit.wav")
            .path
    )
}
```

Also prove malformed names use the modification date in the current calendar, identical existing hashes return `.reuseExisting`, different hashes return a filename with `__<8 hex>` before `.wav`, and path traversal is impossible.

- [ ] **Step 6: Implement destination planning without overwrite**

```swift
public enum ExistingDestination: Sendable {
    case none
    case sameContent
    case differentContent
}

public struct DestinationPlan: Sendable {
    public enum Action: Sendable { case create, reuseExisting }
    public let action: Action
    public let finalURL: URL
    public let temporaryURL: URL
}
```

Temporary names use `.<final-name>.<UUID>.partial` in the same directory. Final names use the original last path component; a different-content collision inserts the first eight lowercase digest characters. Create directories only under the standardized destination root.

- [ ] **Step 7: Run focused and complete tests**

```bash
swift test --filter DJIRecordingNameTests
swift test --filter RecordingScannerTests
swift test --filter DestinationPlannerTests
swift test
```

Expected: all tests pass without accessing `/Volumes`.

- [ ] **Step 8: Commit recording discovery and layout policy**

```bash
git add Sources/DJIMicBackupCore/Discovery Sources/DJIMicBackupCore/Backup/FileMetadata.swift Sources/DJIMicBackupCore/Backup/DestinationPlanner.swift Tests/DJIMicBackupCoreTests
git commit -m "feat: discover stable DJI recordings"
```

---

### Task 4: Persist pairing, verified backups, run history, and deletion state in SQLite

**Files:**
- Create: `Sources/DJIMicBackupCore/Persistence/LedgerModels.swift`
- Create: `Sources/DJIMicBackupCore/Persistence/SQLiteConnection.swift`
- Create: `Sources/DJIMicBackupCore/Persistence/BackupLedger.swift`
- Create: `Sources/DJIMicBackupCore/Support/AppPaths.swift`
- Create: `Tests/DJIMicBackupCoreTests/BackupLedgerTests.swift`

**Interfaces:**
- Consumes: `PairedDevice`, `RecordingCandidate`, `PerDeviceOutcome`.
- Produces: `BackupLedgerProtocol`, SQLite-backed `BackupLedger`, `VerifiedRecording`, `RunRecord`, and `DeletionSnapshot`.

- [ ] **Step 1: Write transactional ledger tests against temporary SQLite files**

Tests must establish:

- Migration creates schema version 1 exactly once and enables foreign keys plus WAL journaling.
- UUID and transmitter label uniqueness reject conflicting pairings.
- A verified recording is found by device UUID plus relative path, size, modification timestamp, and content digest.
- Interrupted run rows remain recoverable and can be marked failed on next launch.
- Complete-snapshot eligibility requires set equality between all currently scanned WAVs and verified current entries.
- Destination missing or digest mismatch revokes eligibility.
- Every successful source unlink can be recorded independently for partial-delete recovery.
- Reopening the database preserves all committed state.

- [ ] **Step 2: Run the ledger tests and verify failure**

```bash
swift test --filter BackupLedgerTests
```

Expected: compilation fails because `BackupLedger` and its models do not exist.

- [ ] **Step 3: Create the schema and migration transaction**

Use this schema without storing audio content:

```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;

CREATE TABLE schema_version (
  version INTEGER NOT NULL
);

CREATE TABLE paired_devices (
  volume_uuid TEXT PRIMARY KEY,
  transmitter_label TEXT NOT NULL UNIQUE CHECK (transmitter_label IN ('TX01','TX02')),
  media_name TEXT NOT NULL,
  capacity_bytes INTEGER NOT NULL,
  paired_at REAL NOT NULL
);

CREATE TABLE backup_runs (
  id TEXT PRIMARY KEY,
  volume_uuid TEXT NOT NULL REFERENCES paired_devices(volume_uuid),
  state TEXT NOT NULL,
  started_at REAL NOT NULL,
  finished_at REAL,
  discovered_count INTEGER NOT NULL DEFAULT 0,
  verified_count INTEGER NOT NULL DEFAULT 0,
  verified_bytes INTEGER NOT NULL DEFAULT 0,
  error_summary TEXT
);

CREATE TABLE recordings (
  id TEXT PRIMARY KEY,
  volume_uuid TEXT NOT NULL REFERENCES paired_devices(volume_uuid),
  source_relative_path TEXT NOT NULL,
  source_size INTEGER NOT NULL,
  source_mtime REAL NOT NULL,
  source_sha256 TEXT NOT NULL,
  destination_relative_path TEXT NOT NULL,
  verified_at REAL NOT NULL,
  deleted_at REAL,
  delete_error TEXT,
  UNIQUE (volume_uuid, source_relative_path, source_size, source_mtime, source_sha256)
);

CREATE INDEX recordings_by_source
ON recordings(volume_uuid, source_relative_path);
```

Every migration runs under `BEGIN IMMEDIATE` and either commits fully or rolls back.

- [ ] **Step 4: Implement a narrow SQLite wrapper**

`SQLiteConnection` owns one `OpaquePointer`, checks every return code, finalizes every statement with `defer`, binds all dynamic values, and maps errors into:

```swift
public enum SQLiteFailure: Error, Equatable {
    case open(code: Int32, message: String)
    case prepare(code: Int32, message: String)
    case bind(code: Int32, message: String)
    case step(code: Int32, message: String)
    case execute(code: Int32, message: String)
    case corrupt(message: String)
}
```

Do not build a general ORM. Implement only `execute`, `prepare`, transaction, and typed column helpers required by the ledger.

- [ ] **Step 5: Implement the actor-isolated ledger interface**

Define the persisted values with stable names used by Tasks 5 and 6:

```swift
public struct VerifiedRecording: Identifiable, Equatable, Sendable {
    public let id: UUID
    public let volumeUUID: UUID
    public let sourceRelativePath: String
    public let sourceSize: Int64
    public let sourceModificationDate: Date
    public let sourceSHA256: String
    public let destinationRelativePath: String
    public let verifiedAt: Date
    public let deletedAt: Date?
}

public enum RunState: String, Codable, Sendable {
    case running
    case succeeded
    case partialFailure
    case failed
    case interrupted
}

public struct RunRecord: Identifiable, Equatable, Sendable {
    public let id: UUID
    public let volumeUUID: UUID
    public let state: RunState
    public let startedAt: Date
    public let finishedAt: Date?
}

public struct DeletionSnapshot: Equatable, Sendable {
    public let device: TrustedMountedDevice
    public let recordings: [VerifiedRecording]
}
```

```swift
public protocol BackupLedgerProtocol: DeviceRegistryProtocol, Sendable {
    func beginRun(device: PairedDevice, at date: Date) async throws -> UUID
    func finishRun(id: UUID, outcome: PerDeviceOutcome, at date: Date) async throws
    func verifiedRecord(matching candidate: RecordingCandidate) async throws -> VerifiedRecording?
    func recordVerified(_ recording: VerifiedRecording) async throws
    func completeSnapshot(
        device: TrustedMountedDevice,
        current: [RecordingCandidate]
    ) async throws -> DeletionSnapshot?
    func recordDeletion(recordingID: UUID, at date: Date) async throws
    func recordDeletionFailure(recordingID: UUID, message: String) async throws
    func recoverInterruptedRuns(at date: Date) async throws -> [RunRecord]
}
```

The concrete `BackupLedger` is an actor, making one SQLite connection the sole serialization point. Store destination paths relative to the configured root so moving the root does not silently create absolute-path authority.

- [ ] **Step 6: Add application-support path creation and corruption behavior**

`AppPaths` resolves `~/Library/Application Support/DJI Mic Backup/backup.sqlite` through Foundation's application-support directory API and creates the containing directory with user-only default permissions. If SQLite reports corruption, move the database and its `-wal`/`-shm` siblings into a timestamped `Corrupt` subdirectory, create a fresh ledger, revoke all deletion eligibility, and surface `destination re-index required`; never infer deletion permission from a damaged database.

- [ ] **Step 7: Run persistence tests twice**

```bash
swift test --filter BackupLedgerTests
swift test
swift test
```

Expected: both complete-suite runs pass, demonstrating cleanup and deterministic reopening.

- [ ] **Step 8: Commit the durable ledger**

```bash
git add Sources/DJIMicBackupCore/Persistence Sources/DJIMicBackupCore/Support/AppPaths.swift Tests/DJIMicBackupCoreTests/BackupLedgerTests.swift
git commit -m "feat: persist verified backup ledger"
```

---

### Task 5: Stream, synchronize, hash, finalize, deduplicate, and recover backups

**Files:**
- Create: `Sources/DJIMicBackupCore/Backup/SHA256FileHasher.swift`
- Create: `Sources/DJIMicBackupCore/Backup/SafeFileCopier.swift`
- Create: `Sources/DJIMicBackupCore/Backup/BackupCoordinator.swift`
- Create: `Sources/DJIMicBackupCore/Support/PrivateLogger.swift`
- Create: `Tests/DJIMicBackupCoreTests/SafeFileCopierTests.swift`
- Create: `Tests/DJIMicBackupCoreTests/BackupCoordinatorTests.swift`

**Interfaces:**
- Consumes: trusted devices, scanner, planner, ledger, `Clock`.
- Produces: `FileHashing`, `SafeCopying`, `BackupCoordinating`, `BackupEvent`, and verified `PerDeviceOutcome` values.

- [ ] **Step 1: Write streaming-hash and verified-copy failure tests**

Cover a multi-megabyte file so hashing cannot rely on loading the whole file into memory. Test:

- Source and destination digests match for a normal copy.
- Destination bytes intentionally mutated before verification produce `hashMismatch` and no final file.
- Source size or modification time changed during the copy produces `sourceChanged` and no final file.
- An injected write failure leaves the source and any pre-existing final file untouched.
- A temporary file is always inside the final directory and ends in `.partial`.
- `FileHandle.synchronize()` failure prevents finalization.

- [ ] **Step 2: Implement incremental SHA-256**

```swift
public struct ContentDigest: Hashable, Codable, Sendable {
    public let hex: String
}

public protocol FileHashing: Sendable {
    func sha256(of url: URL) async throws -> ContentDigest
}
```

Read 1 MiB chunks from `FileHandle`, call `CryptoKit.SHA256.update(data:)`, and finalize to lowercase hex. Check task cancellation between chunks. Never log the URL or digest publicly.

- [ ] **Step 3: Implement safe copy finalization**

The core operation has this contract:

```swift
public protocol SafeCopying: Sendable {
    func copyAndVerify(
        candidate: RecordingCandidate,
        expectedSourceDigest: ContentDigest,
        plan: DestinationPlan
    ) async throws -> VerifiedCopy
}

public struct VerifiedCopy: Equatable, Sendable {
    public let destinationURL: URL
    public let sizeBytes: Int64
    public let digest: ContentDigest
}
```

Implementation order is fixed:

1. Capture source regular-file metadata immediately before opening.
2. Create the temporary destination with exclusive creation; never truncate an existing path.
3. Stream 1 MiB chunks, updating the source hash while writing.
4. Call `synchronize()` and `close()` on the destination handle.
5. Re-read source metadata and require exact size and modification-time equality.
6. Require the streamed source hash to equal `expectedSourceDigest`.
7. Hash the temporary destination independently and require the same digest and byte count.
8. Use `FileManager.moveItem(at:to:)` within the same directory; if a final file appeared concurrently, compare it and deduplicate or choose the hash-suffixed path without overwrite.
9. Return `VerifiedCopy`; on any pre-finalization failure remove only the operation's UUID-named temporary file.

- [ ] **Step 4: Write coordinator tests for full runs and interruption**

Use fake scanner, ledger, capacity provider, copier, and event sink to prove:

1. Empty transmitter ends successfully with zero copied files.
2. Required bytes plus 10 GiB reserve exceeding available capacity performs no copy.
3. New recordings are copied in stable relative-path order and recorded only after final verification.
4. Already verified recordings with intact destinations are re-verified but not recopied.
5. Missing or changed destinations force a safe recopy and revoke old deletion eligibility.
6. One failed file yields `.partialFailure`, keeps successful verified files, and disables the complete snapshot.
7. Two concurrent mount notifications for the same UUID coalesce to one run.
8. TX01 and TX02 may run independently; one failure does not cancel the other.
9. Unmount cancellation leaves sources intact and emits an actionable outcome.
10. A final file left before ledger commit is reconciled by digest on the next run.

- [ ] **Step 5: Implement capacity preflight and the backup actor**

Use one event vocabulary from core to UI:

```swift
public enum BackupEvent: Sendable {
    case phaseChanged(volumeUUID: UUID, phase: BackupPhase)
    case progress(volumeUUID: UUID, completed: Int, total: Int, bytes: Int64)
    case finished(volumeUUID: UUID, outcome: PerDeviceOutcome)
}
```

```swift
public protocol BackupCoordinating: Sendable {
    func handleMountedDevice(_ device: TrustedMountedDevice) async
    func backUpNow(_ devices: [TrustedMountedDevice]) async
    func events() -> AsyncStream<BackupEvent>
    func isRunning(volumeUUID: UUID) async -> Bool
}
```

The actor maintains `[UUID: Task<Void, Never>]`. For each run it scans, asks the ledger which candidates already have verified entries, sums only bytes that require a new copy, reads `.volumeAvailableCapacityForImportantUsageKey`, and refuses the run unless `available >= required + 10 * 1024 * 1024 * 1024`. It emits phase and progress events, finalizes the run row, and removes its task entry in `defer`.

If an existing record is found, hash the current destination before counting it as verified. If it is missing or mismatched, do not grant deletion eligibility and run the normal safe-copy path.

- [ ] **Step 6: Add crash and stale-temporary reconciliation**

At launch, mark ledger runs without `finished_at` as interrupted. For each `.partial` file owned by this app, remove it only when its UUID is not an active operation and its modification date is older than one hour. For an unledgered final file at a deterministic destination, compare its digest to the current source; identical content is adopted into the ledger, while different content follows collision rules.

- [ ] **Step 7: Protect diagnostic logging**

Create category loggers for `device`, `scan`, `backup`, `delete`, and `ui`. Counts, byte totals, phases, and static error codes may be public. Every filename, source path, destination path, UUID, and hash must use `privacy: .private` or remain absent.

- [ ] **Step 8: Run core safety tests and commit**

```bash
swift test --filter SafeFileCopierTests
swift test --filter BackupCoordinatorTests
swift test
git add Sources/DJIMicBackupCore/Backup Sources/DJIMicBackupCore/Support/PrivateLogger.swift Tests/DJIMicBackupCoreTests
git commit -m "feat: add verified automatic backups"
```

---

### Task 6: Require a complete re-verified snapshot before manual deletion

**Files:**
- Create: `Sources/DJIMicBackupCore/Deletion/DeletionCoordinator.swift`
- Create: `Tests/DJIMicBackupCoreTests/DeletionCoordinatorTests.swift`

**Interfaces:**
- Consumes: scanner, ledger, hasher, backup-running state, trusted mounted device, `Clock`.
- Produces: `DeletionProposal`, `DeletionOutcome`, and `DeletionCoordinating`.

- [ ] **Step 1: Write adversarial deletion tests before implementation**

Tests must prove deletion is refused when:

- The device UUID matches no active pairing.
- The pairing matches but removable, writable, USB, media-name, or capacity checks fail.
- A backup is active for that transmitter.
- The current scanned WAV set differs by even one relative path from the verified snapshot.
- Any source size, modification date, or digest changed.
- Any destination is missing, outside the configured root, or has a different digest.
- Any source standardized path escapes the mounted-volume root or is a symlink.
- The confirmation proposal was created for a previous mount instance.

Also simulate an unlink failure on the third of five files and assert that two deletions are recorded, three source files remain, processing stops, and the outcome is `.partiallyDeleted` with no false success.

- [ ] **Step 2: Run the deletion tests and verify failure**

```bash
swift test --filter DeletionCoordinatorTests
```

Expected: compilation fails because deletion types do not exist.

- [ ] **Step 3: Define a proposal that cannot itself authorize deletion**

```swift
public struct DeletionProposal: Identifiable, Sendable {
    public let id: UUID
    public let volumeUUID: UUID
    public let mountURL: URL
    public let label: TransmitterLabel
    public let recordIDs: [UUID]
    public let fileCount: Int
    public let totalBytes: Int64
    public let destinationRoot: URL
    public let createdAt: Date
}

public enum DeletionRefusal: String, Equatable, Sendable {
    case deviceNotTrusted
    case deviceRemounted
    case backupInProgress
    case snapshotIncomplete
    case sourceChanged
    case destinationMissing
    case destinationChanged
    case pathOutsideTrustedRoot
}

public enum DeletionOutcome: Equatable, Sendable {
    case deleted(count: Int, bytes: Int64)
    case refused(reason: DeletionRefusal)
    case partiallyDeleted(deleted: Int, remaining: Int, message: String)
}
```

The proposal is display data. `executeConfirmed(_:)` must derive authority again from the live volume, current scan, ledger, and hashes.

- [ ] **Step 4: Implement whole-snapshot preflight**

```swift
public protocol DeletionCoordinating: Sendable {
    func prepare(for device: TrustedMountedDevice) async throws -> DeletionProposal?
    func executeConfirmed(_ proposal: DeletionProposal) async -> DeletionOutcome
}
```

Before the first unlink, `executeConfirmed` performs this exact order:

1. Resolve the currently mounted volume by UUID and re-run `DeviceMatcher`.
2. Require the current mount URL to equal the proposal's standardized mount URL.
3. Require no active backup task for that UUID.
4. Re-scan stable WAVs and require relative-path set equality with the proposal and ledger snapshot.
5. Require every source to be regular, non-symlink, within root, and metadata-identical.
6. Hash every source and destination and require both to equal the stored digest.
7. Require every destination standardized path to remain under the configured destination root.
8. Only after all checks pass, unlink sources in sorted relative-path order.

Do not delete source directories after the files are removed.

- [ ] **Step 5: Persist and report every unlink result**

After each successful unlink, immediately commit `deleted_at` for that record. On the first unlink error, immediately commit `delete_error`, stop, and return `.partiallyDeleted`. A later run treats already missing source files with recorded `deleted_at` as completed, but never treats an unrecorded missing source as proof of success.

- [ ] **Step 6: Run all deletion and regression tests**

```bash
swift test --filter DeletionCoordinatorTests
swift test --filter BackupCoordinatorTests
swift test
```

Expected: all tests pass, including zero calls to `removeItem` for every preflight refusal.

- [ ] **Step 7: Commit manual deletion as a separately reviewable safety boundary**

```bash
git add Sources/DJIMicBackupCore/Deletion Tests/DJIMicBackupCoreTests/DeletionCoordinatorTests.swift
git commit -m "feat: add confirmed verified-source deletion"
```

---

### Task 7: Connect the menu bar UX, setup flow, notifications, and login launch

**Files:**
- Create: `Sources/DJIMicBackupApp/AppModel.swift`
- Modify: `Sources/DJIMicBackupApp/DJIMicBackupApp.swift`
- Create: `Sources/DJIMicBackupApp/MenuBarContent.swift`
- Create: `Sources/DJIMicBackupApp/SetupView.swift`
- Create: `Sources/DJIMicBackupApp/SetupWindowController.swift`
- Create: `Sources/DJIMicBackupApp/DestinationAccess.swift`
- Create: `Sources/DJIMicBackupApp/DeletionConfirmation.swift`
- Create: `Sources/DJIMicBackupApp/NotificationClient.swift`
- Create: `Sources/DJIMicBackupApp/LoginItemClient.swift`
- Create: `Tests/DJIMicBackupAppTests/AppModelTests.swift`

**Interfaces:**
- Consumes: all core protocols and outcomes.
- Produces: the user-visible menu bar app and main-actor `AppModel`.

- [ ] **Step 1: Write AppModel tests with protocol fakes**

Assert the Korean user-facing states and action gates:

| Core phase | Menu title | Symbol | Delete action |
|---|---|---|---|
| idle | `대기 중` | `externaldrive` | disabled |
| deviceDetected | `마이크 확인 중` | `externaldrive.badge.plus` | disabled |
| backingUp | `백업 중…` | `arrow.down.circle` | disabled |
| verifiedDeletionPending | `백업 완료 · 삭제 대기` | `checkmark.circle.fill` | enabled |
| partialFailure | `일부 파일 백업 실패` | `exclamationmark.triangle.fill` | disabled for affected transmitter |
| error | `백업 오류` | `xmark.circle.fill` | disabled |

Tests also prove duplicate device events do not duplicate UI rows, notification denial leaves menu status intact, destination loss produces an error without calling backup, and clicking delete first requests a proposal and never invokes execution until confirmation returns true.

- [ ] **Step 2: Implement `AppModel` as the only UI state owner**

Define the per-device presentation explicitly:

```swift
struct DevicePresentation: Equatable, Sendable {
    let label: TransmitterLabel
    let volumeName: String
    let phase: BackupPhase
    let verifiedCount: Int
    let verifiedBytes: Int64
    let detail: String?
}
```

```swift
@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var phase: BackupPhase = .idle
    @Published private(set) var devices: [TransmitterLabel: DevicePresentation] = [:]
    @Published private(set) var lastOutcome: BackupRunOutcome?
    @Published private(set) var setupRequired = true
    @Published private(set) var statusMessage = "대기 중"

    var symbolName: String {
        switch phase {
        case .idle: "externaldrive"
        case .deviceDetected: "externaldrive.badge.plus"
        case .backingUp: "arrow.down.circle"
        case .verifiedDeletionPending: "checkmark.circle.fill"
        case .partialFailure: "exclamationmark.triangle.fill"
        case .error: "xmark.circle.fill"
        }
    }

    func start() async
    func backUpNow() async
    func requestDeletion(for label: TransmitterLabel) async
    func openDestination()
    func setLaunchAtLogin(_ enabled: Bool) async
}
```

`start()` opens the ledger, recovers interrupted runs, requests notification authorization in explanatory context, restores the destination, begins Disk Arbitration monitoring, and consumes device and backup event streams. All filesystem work stays off the main actor inside core actors.

Implement `AppModel.live()` in the same file as the single composition root. It creates `AppPaths`, `BackupLedger`, `DeviceMatcher`, `DiskArbitrationMonitor`, `RecordingScanner` with a two-second settling interval, `DestinationPlanner`, `SHA256FileHasher`, `SafeFileCopier`, `BackupCoordinator`, `DeletionCoordinator`, `DestinationAccess`, `NotificationClient`, and `LoginItemClient`, then injects them into the model initializer. Tests call the initializer directly with fakes and never call `live()`.

- [ ] **Step 3: Implement first-run destination approval and pairing**

Use `NSOpenPanel` configured for one directory and preselect `/Users/channprj/Documents/DJI-Mic-Mini-2S`. The user must confirm that folder before automatic backup begins. Persist the resolved folder URL and a bookmark in `UserDefaults`; if bookmark restoration or filesystem access fails later, stop backup and present `백업 폴더 접근 권한이 필요합니다` with a reselect action.

The pairing view lists only unpaired physical volumes matching the non-authoritative DJI shape. Infer `TX01` or `TX02` only when all observed valid filenames agree. The user sees the proposed labels and confirms both. Save through `DeviceRegistryProtocol`; do not hardcode current UUIDs in source.

- [ ] **Step 4: Build a window-style menu extra with exact actions**

```swift
@main
struct DJIMicBackupApp: App {
    @StateObject private var model = AppModel.live()

    var body: some Scene {
        MenuBarExtra {
            MenuBarContent(model: model)
                .frame(width: 320)
                .task { await model.start() }
        } label: {
            Label("DJI Mic Backup", systemImage: model.symbolName)
                .accessibilityLabel(model.statusMessage)
        }
        .menuBarExtraStyle(.window)
    }
}
```

`MenuBarContent` shows status, per-device file counts and bytes, last completion time, and these actions in order: `지금 백업`, `검증된 원본 삭제…`, `백업 폴더 열기`, `로그인 시 실행`, and `종료`. Disable destructive actions while scanning, copying, unpaired, disconnected, incomplete, or errored.

- [ ] **Step 5: Implement explicit destructive confirmation**

`DeletionConfirmation` uses `NSAlert` on the main actor. The informative text includes transmitter label, exact file count, formatted bytes, and destination path. Buttons are `원본 삭제` and `취소`, with cancel as the default keyboard action. The alert never exposes a callable closure that bypasses `DeletionCoordinator.executeConfirmed`.

- [ ] **Step 6: Implement local notifications as secondary status**

Request only `.alert` and `.sound`. Post unique local notification requests for:

- `backup-success-<run-id>`: `백업 완료` and `<TX labels> · <count>개 · <bytes> · 원본 삭제 대기`.
- `backup-empty-<run-id>`: `새 녹음 없음` and `연결된 마이크를 확인했습니다.`
- `backup-failure-<run-id>`: `백업 확인 필요` and a non-sensitive reason plus `원본은 삭제되지 않았습니다.`
- `delete-success-<proposal-id>`: `원본 삭제 완료` and `<label> · <count>개 · <bytes>`.
- `delete-partial-<proposal-id>`: `일부 원본 삭제 실패` and deleted/remaining counts.

When authorization is denied, do not nag repeatedly; show `알림 꺼짐` in the menu and keep the persistent status complete.

- [ ] **Step 7: Implement login launch with system authorization state**

`LoginItemClient` wraps `SMAppService.mainApp`. On enable, call `register()` and map `enabled`, `requiresApproval`, `notRegistered`, and `notFound` into UI state. If approval is required or registration is denied, show a button that calls `SMAppService.openSystemSettingsLoginItems()`. On disable, call `unregister()`.

The first-run setup recommends and defaults to enabling launch at login, but the app must show the actual system status rather than assuming registration succeeded.

- [ ] **Step 8: Run model tests, accessibility checks, and commit**

```bash
swift test --filter AppModelTests
swift test
swift build --product DJIMicBackup
git add Sources/DJIMicBackupApp Tests/DJIMicBackupAppTests
git commit -m "feat: add menu bar backup workflow"
```

Manual review must confirm VoiceOver labels for the status icon and every action, keyboard access to setup and confirmation, high-contrast system colors, and no status conveyed by color alone.

---

### Task 8: Package, install, document, and prove the complete real-device chain

**Files:**
- Create: `Packaging/Info.plist`
- Create: `scripts/build-app.sh`
- Create: `scripts/install-app.sh`
- Create: `scripts/smoke-test.sh`
- Create: `README.md`
- Modify: `docs/superpowers/specs/2026-08-09-dji-mic-mini-auto-backup-design.md` only if implementation evidence requires a factual correction; do not rewrite settled behavior.

**Interfaces:**
- Consumes: the release executable and all accepted behavior.
- Produces: `build/DJI Mic Backup.app`, a recoverable user-local install, repeatable smoke proof, and operating documentation.

- [ ] **Step 1: Create a GUI-agent plist without sandbox or privileged-helper claims**

`Packaging/Info.plist` contains:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>ko</string>
  <key>CFBundleExecutable</key><string>DJIMicBackup</string>
  <key>CFBundleIdentifier</key><string>com.channprj.DJIMicBackup</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>DJI Mic Backup</string>
  <key>CFBundleDisplayName</key><string>DJI Mic Backup</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHumanReadableCopyright</key><string>Copyright © 2026 channprj</string>
</dict>
</plist>
```

Do not enable App Sandbox in v1: persistent automatic access to two removable volumes is the central behavior, and this local non-App-Store utility has no network or privileged helper surface. Destination selection still uses `NSOpenPanel` for explicit user intent and protected-folder access.

- [ ] **Step 2: Implement deterministic bundle assembly**

`scripts/build-app.sh` must use `set -euo pipefail`, resolve the repository root from the script location, run release SwiftPM build, assemble `build/DJI Mic Backup.app/Contents/{MacOS,Resources}`, copy the executable and plist, validate the plist with `plutil -lint`, ad-hoc sign with the fixed identifier, and verify the signature:

```bash
swift build --configuration release --product DJIMicBackup
codesign --force --sign - --identifier com.channprj.DJIMicBackup "build/DJI Mic Backup.app"
codesign --verify --deep --strict --verbose=2 "build/DJI Mic Backup.app"
```

The script must never write to `/Volumes` or the configured backup destination.

- [ ] **Step 3: Implement a recoverable per-user installer**

Install to `$HOME/Applications/DJI Mic Backup.app`. Build first, stage the new bundle beside the destination, verify its signature, move an existing installed bundle to a timestamped directory under `$HOME/Library/Caches/DJI Mic Backup/Install Backups/`, and then atomically rename the staged bundle into place. Never recursively delete the previous installed bundle. Print the backup path when replacement occurred.

After install, launch with:

```bash
open "$HOME/Applications/DJI Mic Backup.app"
```

- [ ] **Step 4: Create non-destructive automated smoke checks**

`scripts/smoke-test.sh` must run:

1. `swift test`.
2. Release build and bundle assembly.
3. `plutil -lint` and assertions for bundle ID, `LSUIElement=true`, and minimum system version.
4. `codesign --verify --deep --strict`.
5. Launch the built app and wait up to ten seconds for the process.
6. Verify the process remains running and terminate only that exact built process gracefully.
7. Inspect the source tree for forbidden network frameworks and hardcoded current transmitter UUIDs.

The smoke script must not mount, copy from, rename, or delete anything on the real DJI volumes.

- [ ] **Step 5: Perform backup-only hardware acceptance with the current production inventory**

Before the app touches real recordings, make a read-only inventory of both mounted volumes and record file counts and byte totals outside the source volumes. Then:

1. Pair both current volumes and confirm filename inference maps them to TX01 and TX02.
2. Confirm the selected destination is `/Users/channprj/Documents/DJI-Mic-Mini-2S`.
3. Confirm capacity preflight leaves the 10 GiB reserve.
4. Run automatic backup without approving deletion.
5. Independently calculate source and destination SHA-256 for every inventoried production file and require exact one-to-one equality.
6. Reconnect the case and verify zero duplicate destination files are created.
7. Confirm the menu and notification show the same verified count as the pre-run inventory and deletion pending.

The plan-time count is eleven, but the pre-run inventory is authoritative if new recordings appear before implementation. Do not click the deletion action for any production recording during acceptance.

- [ ] **Step 6: Prove destructive behavior with an isolated disposable volume fixture**

Create a temporary FAT32 disk image or a temporary-directory filesystem adapter containing only disposable DJI-shaped WAV fixtures. Exercise these cases:

1. Back up disposable TX01 and TX02 files and independently verify hashes.
2. Cancel the first deletion confirmation and prove all sources remain.
3. Modify or replace one disposable source after backup and prove whole-snapshot preflight refuses deletion before any unlink.
4. Restore a clean disposable snapshot, confirm deletion, and prove exactly the verified fixture WAV files are removed while hidden and unknown fixture files remain.
5. Inject failure on the third unlink and prove the partial outcome and ledger state are exact.
6. Simulate disconnect during a larger disposable copy and prove the source remains and the next mount recovers.

Do not run deletion acceptance on the current physical transmitters while any production recording remains. Physical deletion proof is optional and requires a separate, user-approved test state in which every WAV on a transmitter is disposable and the read-only preflight inventory proves that fact. Otherwise report that deletion is integration-proven but not destructively exercised on the production hardware.

- [ ] **Step 7: Verify login, restart, privacy, and installed-artifact parity**

1. Enable login launch and confirm `SMAppService.mainApp.status` is enabled or explicitly awaiting System Settings approval.
2. Log out and back in, then confirm the installed menu bar app starts without a terminal.
3. Deny notifications once and confirm menu status remains complete; re-enable and confirm success notification delivery.
4. Inspect unified logs and confirm filenames, paths, UUIDs, and hashes are redacted.
5. Compare the installed executable SHA-256 to the release bundle executable SHA-256.
6. Re-run `swift test` and `scripts/smoke-test.sh` after installation.

- [ ] **Step 8: Write the operating guide**

`README.md` must document:

- Purpose, macOS 13 minimum, and local-only privacy boundary.
- Build, bundle, install, launch, and uninstall instructions.
- First-run destination approval, pairing, notifications, and login-item approval.
- Exact meaning of each menu state.
- Why `NO NAME`, `DJI-MIC-1`, or any volume label alone is not trusted.
- Backup layout and collision behavior.
- The 10 GiB reserve and how to recover from insufficient space.
- Manual deletion contract, whole-snapshot refusal, and unavoidable partial-unlink failure semantics.
- Ledger and corruption recovery location.
- How to read redacted diagnostic logs.
- A warning that production recordings must not be used as deletion test fixtures.

- [ ] **Step 9: Run final gates and commit the installable tool**

```bash
swift test
scripts/smoke-test.sh
git status --short
git add Packaging scripts README.md docs Sources Tests Package.swift .gitignore
git commit -m "docs: add install and hardware verification guide"
git status --short --branch
```

Expected: tests and smoke checks pass, the app bundle verifies, the installed artifact matches the release executable, real-device backup proof passes, any destructive hardware proof limit is explicit, and the working tree is clean.

---

## Requirement-to-Task Coverage

| Design requirement | Owning tasks |
|---|---|
| Login-launched menu bar app with persistent status | Tasks 1, 7, 8 |
| Detect two identically named/mutably named physical volumes | Tasks 2, 7, 8 |
| Automatic stable-WAV discovery | Tasks 3, 5 |
| Deterministic dated TX01/TX02 layout | Task 3 |
| SHA-256 and byte-count verification before success | Task 5 |
| Deduplication, collision preservation, reconnect recovery | Tasks 3, 4, 5 |
| Clear success, empty, partial, and failure notification | Task 7 |
| Manual whole-snapshot deletion confirmation | Tasks 6, 7 |
| Fail closed on identity, content, path, or permission ambiguity | Tasks 2, 5, 6 |
| Keep hidden and unknown source content untouched | Tasks 3, 6 |
| Local-only privacy and redacted diagnostics | Tasks 5, 7, 8 |
| Capacity reserve and actionable disk-full behavior | Tasks 5, 7, 8 |
| Real-device and installed-artifact proof | Task 8 |

## Risk Register

1. **Volume UUID changes after reformatting.** Expected behavior is loss of trust, not heuristic auto-repair. The UI requires pairing again and deletion stays disabled.
2. **Disk Arbitration event arrives before mount completion.** Register description-change callbacks and emit only after `VolumePath` exists.
3. **Destination has only about 18 GiB free.** The fixed 10 GiB reserve means a run needing more than roughly 8 GiB will intentionally stop until the user frees space.
4. **Multi-file deletion is not atomic on FAT32.** Whole-snapshot preflight prevents stale or incomplete authority, while per-file ledger commits and stop-on-first-error make an unlink-time partial outcome recoverable and honest.
5. **Ad-hoc signing can cause macOS permission identity churn after rebuilds.** Run final hardware acceptance against the installed final build. Developer ID signing and notarization are follow-up distribution work, not a prerequisite for this single-Mac utility.
6. **App termination or disconnect during copy.** UUID-owned `.partial` files, source metadata re-checks, task cancellation, and launch reconciliation prevent false success or source deletion.
7. **Ledger corruption.** Quarantine the corrupt files, create a fresh ledger, and revoke deletion until destination re-indexing; never reconstruct deletion authority from filenames alone.
8. **Notification permission denied.** The menu bar state remains canonical and exposes the last result; notification delivery is additive.

## Definition of Done

- All Swift tests pass repeatedly from a clean checkout using Command Line Tools.
- `scripts/build-app.sh` produces a valid signed `DJI Mic Backup.app` with the expected bundle metadata.
- The installed app launches at login after macOS approval and remains usable entirely from the menu bar.
- Both current transmitter UUIDs can be paired and are correctly inferred as TX01 and TX02 while label/path changes do not affect identity.
- Every file in the pre-run production inventory (eleven at plan time) is backed up with independent source/destination SHA-256 parity, zero duplicate files on reconnect, and no source deletion during backup acceptance.
- Manual deletion is unavailable for incomplete snapshots and requires confirmation plus complete live re-verification.
- Disposable-fixture deletion tests pass, or the physical deletion proof limitation is explicitly reported without risking production recordings.
- Source files remain intact across scan changes, insufficient space, permission errors, write errors, hash mismatches, app interruption, and disconnects.
- The destination layout, menu states, notifications, ledger recovery, logging privacy, install/rollback, and operating instructions match the approved design.
- The final Git working tree is clean with reviewable task-level commits.

## Official References

- [Apple MenuBarExtra documentation](https://developer.apple.com/documentation/swiftui/menubarextra)
- [Apple Disk Arbitration appeared callback](https://developer.apple.com/documentation/diskarbitration/1492707-daregisterdiskappearedcallback)
- [Apple Disk Arbitration description constants](https://developer.apple.com/documentation/diskarbitration/diskarbitration-constants)
- [Apple SMAppService documentation](https://developer.apple.com/documentation/servicemanagement/smappservice)
- [Apple SMAppService register contract](https://developer.apple.com/documentation/servicemanagement/smappservice/register%28%29)
- [Apple UserNotifications authorization contract](https://developer.apple.com/documentation/usernotifications/unusernotificationcenter/requestauthorization%28options%3Acompletionhandler%3A%29)
- [Apple CryptoKit SHA256 documentation](https://developer.apple.com/documentation/cryptokit/sha256)
- [Apple FileHandle synchronize contract](https://developer.apple.com/documentation/foundation/filehandle/synchronize%28%29)
- [Apple FileManager move contract](https://developer.apple.com/documentation/foundation/filemanager/moveitem%28atpath%3Atopath%3A%29)
- [Apple OSLog privacy documentation](https://developer.apple.com/documentation/os/oslogprivacy)
- [Apple macOS sandbox file-access guidance](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox)
