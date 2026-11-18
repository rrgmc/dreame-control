// Enum tables ported from Tasshack dreame-vacuum `dreame/types.py` (MIT License,
// Copyright (c) 2022 Tasshack), tag v2.0.3. Generated, then kept by hand.
// Labels are hints. Codes not listed decode to `Open::Unknown`.

open_enum! {
    /// Robot state, property 2.1 (`DreameVacuumState`).
    State {
        Sweeping = 1 => "sweeping",
        Idle = 2 => "idle",
        Paused = 3 => "paused",
        Error = 4 => "error",
        Returning = 5 => "returning",
        Charging = 6 => "charging",
        Mopping = 7 => "mopping",
        Drying = 8 => "drying",
        Washing = 9 => "washing",
        ReturningToWash = 10 => "returning to wash",
        Building = 11 => "building",
        SweepingAndMopping = 12 => "sweeping and mopping",
        ChargingCompleted = 13 => "charging completed",
        Upgrading = 14 => "upgrading",
        CleanSummon = 15 => "clean summon",
        StationReset = 16 => "station reset",
        ReturningInstallMop = 17 => "returning install mop",
        ReturningRemoveMop = 18 => "returning remove mop",
        WaterCheck = 19 => "water check",
        CleanAddWater = 20 => "clean add water",
        WashingPaused = 21 => "washing paused",
        AutoEmptying = 22 => "auto emptying",
        RemoteControl = 23 => "remote control",
        SmartCharging = 24 => "smart charging",
        SecondCleaning = 25 => "second cleaning",
        HumanFollowing = 26 => "human following",
        SpotCleaning = 27 => "spot cleaning",
        ReturningAutoEmpty = 28 => "returning auto empty",
        WaitingForTask = 29 => "waiting for task",
        StationCleaning = 30 => "station cleaning",
        ReturningToDrain = 31 => "returning to drain",
        Draining = 32 => "draining",
        AutoWaterDraining = 33 => "auto water draining",
        Emptying = 34 => "emptying",
        DustBagDrying = 35 => "dust bag drying",
        DustBagDryingPaused = 36 => "dust bag drying paused",
        HeadingToExtraCleaning = 37 => "heading to extra cleaning",
        ExtraCleaning = 38 => "extra cleaning",
        FindingPetPaused = 95 => "finding pet paused",
        FindingPet = 96 => "finding pet",
        Shortcut = 97 => "shortcut",
        Monitoring = 98 => "monitoring",
        MonitoringPaused = 99 => "monitoring paused",
        InitialDeepCleaning = 101 => "initial deep cleaning",
        InitialDeepCleaningPaused = 102 => "initial deep cleaning paused",
        Sanitizing = 103 => "sanitizing",
        SanitizingWithDry = 104 => "sanitizing with dry",
        ChangingMop = 105 => "changing mop",
        ChangingMopPaused = 106 => "changing mop paused",
        FloorMaintaining = 107 => "floor maintaining",
        FloorMaintainingPaused = 108 => "floor maintaining paused",
        RemotePickup = 109 => "remote pickup",
        ArrangingItems = 113 => "arranging items",
        PetGuarding = 114 => "pet guarding",
        PetGuardingPaused = 115 => "pet guarding paused",
        InstallingMop = 116 => "installing mop",
        UninstallingMop = 117 => "uninstalling mop",
        IntelligentRecharging = 118 => "intelligent recharging",
        AssistedCleaning = 120 => "assisted cleaning",
        EnteringDock = 121 => "entering dock",
        LeavingDock = 122 => "leaving dock",
        NavigatingToClimber = 140 => "navigating to climber",
        DockingToClimber = 141 => "docking to climber",
        ClimberDocked = 142 => "climber docked",
        ClimberNavigating = 143 => "climber navigating",
        ClimbingStairs = 144 => "climbing stairs",
        ClimbingStairsCompleted = 145 => "climbing stairs completed",
        ClimberAtDock = 146 => "climber at dock",
        ClimberLeavingDock = 147 => "climber leaving dock",
    }
}

open_enum! {
    /// Robot state 2.1 on models without `NEW_STATE` (values above 18 differ).
    StateOld {
        Sweeping = 1 => "sweeping",
        Idle = 2 => "idle",
        Paused = 3 => "paused",
        Error = 4 => "error",
        Returning = 5 => "returning",
        Charging = 6 => "charging",
        Mopping = 7 => "mopping",
        Drying = 8 => "drying",
        Washing = 9 => "washing",
        ReturningToWash = 10 => "returning to wash",
        Building = 11 => "building",
        SweepingAndMopping = 12 => "sweeping and mopping",
        ChargingCompleted = 13 => "charging completed",
        Upgrading = 14 => "upgrading",
        CleanSummon = 15 => "clean summon",
        StationReset = 16 => "station reset",
        ReturningInstallMop = 17 => "returning install mop",
        ReturningRemoveMop = 18 => "returning remove mop",
        RemoteControl = 19 => "remote control",
        CleanAddWater = 20 => "clean add water",
        Monitoring = 21 => "monitoring",
        MonitoringPaused = 22 => "monitoring paused",
        WashingPaused = 23 => "washing paused",
        AutoEmptying = 24 => "auto emptying",
        WaterCheck = 25 => "water check",
        SmartCharging = 26 => "smart charging",
    }
}

open_enum! {
    /// Vacuum status, property 4.1; also the first input of action 4.a1.
    VacuumStatus {
        Idle = 0 => "idle",
        Paused = 1 => "paused",
        Cleaning = 2 => "cleaning",
        BackHome = 3 => "back home",
        PartialCleaning = 4 => "partial cleaning",
        FollowWall = 5 => "follow wall",
        Charging = 6 => "charging",
        Ota = 7 => "ota",
        Fct = 8 => "fct",
        WifiSet = 9 => "wifi set",
        PowerOff = 10 => "power off",
        Factory = 11 => "factory",
        Error = 12 => "error",
        RemoteControl = 13 => "remote control",
        Sleeping = 14 => "sleeping",
        SelfRepair = 15 => "self repair",
        FactoryFuncionTest = 16 => "factory funcion test",
        Standby = 17 => "standby",
        SegmentCleaning = 18 => "segment cleaning",
        ZoneCleaning = 19 => "zone cleaning",
        SpotCleaning = 20 => "spot cleaning",
        FastMapping = 21 => "fast mapping",
        CruisingPath = 22 => "cruising path",
        CruisingPoint = 23 => "cruising point",
        SummonClean = 24 => "summon clean",
        Shortcut = 25 => "shortcut",
        PersonFollow = 26 => "person follow",
        PetGuarding = 27 => "pet guarding",
        AutoArrangement = 28 => "auto arrangement",
        SmartArrangement = 29 => "smart arrangement",
        ZonedArrangement = 30 => "zoned arrangement",
        WaterCheck = 1501 => "water check",
    }
}

open_enum! {
    /// Task status, property 4.7.
    TaskStatus {
        Completed = 0 => "completed",
        AutoCleaning = 1 => "auto cleaning",
        ZoneCleaning = 2 => "zone cleaning",
        SegmentCleaning = 3 => "segment cleaning",
        SpotCleaning = 4 => "spot cleaning",
        FastMapping = 5 => "fast mapping",
        AutoCleaningPaused = 6 => "auto cleaning paused",
        ZoneCleaningPaused = 7 => "zone cleaning paused",
        SegmentCleaningPaused = 8 => "segment cleaning paused",
        SpotCleaningPaused = 9 => "spot cleaning paused",
        MapCleaningPaused = 10 => "map cleaning paused",
        DockingPaused = 11 => "docking paused",
        MoppingPaused = 12 => "mopping paused",
        SegmentMoppingPaused = 13 => "segment mopping paused",
        ZoneMoppingPaused = 14 => "zone mopping paused",
        AutoMoppingPaused = 15 => "auto mopping paused",
        AutoDockingPaused = 16 => "auto docking paused",
        SegmentDockingPaused = 17 => "segment docking paused",
        ZoneDockingPaused = 18 => "zone docking paused",
        CruisingPath = 20 => "cruising path",
        CruisingPathPaused = 21 => "cruising path paused",
        CruisingPoint = 22 => "cruising point",
        CruisingPointPaused = 23 => "cruising point paused",
        SummonCleanPaused = 24 => "summon clean paused",
        ReturningInstallMop = 25 => "returning install mop",
        ReturningRemoveMop = 26 => "returning remove mop",
        StationCleaning = 27 => "station cleaning",
        PetFinding = 30 => "pet finding",
        AutoCleaningWashingPaused = 31 => "auto cleaning washing paused",
        AreaCleaningWashingPaused = 32 => "area cleaning washing paused",
        CustomCleaningWashingPaused = 33 => "custom cleaning washing paused",
        PickingUpItem = 34 => "picking up item",
        PickingUpItemPaused = 35 => "picking up item paused",
        PickingUpItemSuccess = 36 => "picking up item success",
        RemotePickupInitializing = 37 => "remote pickup initializing",
        RemotePickupIdentifing = 38 => "remote pickup identifing",
        ManualRemotePickup = 39 => "manual remote pickup",
        AutomaticRemotePickup = 40 => "automatic remote pickup",
        RemotePickupInProgress = 41 => "remote pickup in progress",
        RemotePickupPaused = 42 => "remote pickup paused",
        PlacingItem = 43 => "placing item",
        PlacingItemPaused = 44 => "placing item paused",
    }
}

open_enum! {
    /// Task type, property 4.58.
    TaskType {
        Idle = 0 => "idle",
        Standard = 1 => "standard",
        StandardPaused = 2 => "standard paused",
        Custom = 3 => "custom",
        CustomPaused = 4 => "custom paused",
        Shortcut = 5 => "shortcut",
        ShortcutPaused = 6 => "shortcut paused",
        Scheduled = 7 => "scheduled",
        ScheduledPaused = 8 => "scheduled paused",
        Smart = 9 => "smart",
        SmartPaused = 10 => "smart paused",
        Partial = 11 => "partial",
        PartialPaused = 12 => "partial paused",
        Summon = 13 => "summon",
        SummonPaused = 14 => "summon paused",
        WaterStain = 15 => "water stain",
        WaterStainPaused = 16 => "water stain paused",
        BoostedEdgeCleaning = 17 => "boosted edge cleaning",
        HairCompressing = 18 => "hair compressing",
        LargeParticleCleaning = 19 => "large particle cleaning",
        IntensiveStainCleaning = 20 => "intensive stain cleaning",
        StainCleaning = 21 => "stain cleaning",
        InitialDeepCleaning = 22 => "initial deep cleaning",
        InitialDeepCleaningPaused = 23 => "initial deep cleaning paused",
        MopPadHeating = 24 => "mop pad heating",
        CleaningAfterMapping = 25 => "cleaning after mapping",
        SmallParticleCleaning = 26 => "small particle cleaning",
        ChangingMop = 30 => "changing mop",
        ChangingMopPaused = 31 => "changing mop paused",
        FloorMaintaining = 32 => "floor maintaining",
        FloorMaintainingPaused = 33 => "floor maintaining paused",
        ArrangingItems = 34 => "arranging items",
        ArrangingItemsPaused = 35 => "arranging items paused",
        IntensiveHairCleaning = 36 => "intensive hair cleaning",
        AccessoryHandling = 37 => "accessory handling",
        IncreasedDrumSpeedCleaning = 38 => "increased drum speed cleaning",
        PressurizedCleaning = 39 => "pressurized cleaning",
        SteamCleaning = 40 => "steam cleaning",
        SteamCleaningPaused = 41 => "steam cleaning paused",
    }
}

open_enum! {
    /// Error code, property 2.2, and the codes in the fault list 4.18. Meanings differ by firmware; labels are hints.
    ErrorCode {
        NoError = 0 => "no error",
        Drop = 1 => "drop",
        Cliff = 2 => "cliff",
        Bumper = 3 => "bumper",
        Gesture = 4 => "gesture",
        BumperRepeat = 5 => "bumper repeat",
        DropRepeat = 6 => "drop repeat",
        OpticalFlow = 7 => "optical flow",
        Box = 8 => "box",
        Tankbox = 9 => "tankbox",
        WaterboxEmpty = 10 => "waterbox empty",
        BoxFull = 11 => "box full",
        Brush = 12 => "brush",
        SideBrush = 13 => "side brush",
        Fan = 14 => "fan",
        LeftWheelMotor = 15 => "left wheel motor",
        RightWheelMotor = 16 => "right wheel motor",
        TurnSuffocate = 17 => "turn suffocate",
        ForwardSuffocate = 18 => "forward suffocate",
        ChargerGet = 19 => "charger get",
        BatteryLow = 20 => "battery low",
        ChargeFault = 21 => "charge fault",
        BatteryPercentage = 22 => "battery percentage",
        Heart = 23 => "heart",
        CameraOcclusion = 24 => "camera occlusion",
        Move = 25 => "move",
        FlowShielding = 26 => "flow shielding",
        InfraredShielding = 27 => "infrared shielding",
        ChargeNoElectric = 28 => "charge no electric",
        BatteryFault = 29 => "battery fault",
        FanSpeedError = 30 => "fan speed error",
        LeftwhellSpeed = 31 => "leftwhell speed",
        RightwhellSpeed = 32 => "rightwhell speed",
        Bmi055Acce = 33 => "bmi055 acce",
        Bmi055Gyro = 34 => "bmi055 gyro",
        Xv7001 = 35 => "xv7001",
        LeftMagnet = 36 => "left magnet",
        RightMagnet = 37 => "right magnet",
        FlowError = 38 => "flow error",
        InfraredFault = 39 => "infrared fault",
        CameraFault = 40 => "camera fault",
        StrongMagnet = 41 => "strong magnet",
        WaterPump = 42 => "water pump",
        Rtc = 43 => "rtc",
        AutoKeyTrig = 44 => "auto key trig",
        P3v3 = 45 => "p3v3",
        CameraIdle = 46 => "camera idle",
        Blocked = 47 => "blocked",
        LdsError = 48 => "lds error",
        LdsBumper = 49 => "lds bumper",
        WaterPump2 = 50 => "water pump 2",
        FilterBlocked = 51 => "filter blocked",
        Edge = 54 => "edge",
        Carpet = 55 => "carpet",
        Laser = 56 => "laser",
        Edge2 = 57 => "edge 2",
        Ultrasonic = 58 => "ultrasonic",
        NoGoZone = 59 => "no go zone",
        Route = 61 => "route",
        Route2 = 62 => "route 2",
        Blocked2 = 63 => "blocked 2",
        Blocked3 = 64 => "blocked 3",
        Restricted = 65 => "restricted",
        Restricted2 = 66 => "restricted 2",
        Restricted3 = 67 => "restricted 3",
        RemoveMop = 68 => "remove mop",
        MopRemoved = 69 => "mop removed",
        MopRemoved2 = 70 => "mop removed 2",
        MopPadStopRotate = 71 => "mop pad stop rotate",
        MopPadStopRotate2 = 72 => "mop pad stop rotate 2",
        MopInstallFailed = 74 => "mop install failed",
        LowBatteryTurnOff = 75 => "low battery turn off",
        DirtyTankNotInstalled = 76 => "dirty tank not installed",
        RobotInHiddenRoom = 78 => "robot in hidden room",
        LdsFailedToLift = 79 => "lds failed to lift",
        RobotStuck = 80 => "robot stuck",
        RobotStuckRepeat = 81 => "robot stuck repeat",
        SlipperyFloor = 82 => "slippery floor",
        UnknownError = 84 => "unknown error",
        CheckMopInstall = 85 => "check mop install",
        DirtyWaterTankFull = 86 => "dirty water tank full",
        RetractableLegStuck = 88 => "retractable leg stuck",
        InternalError = 89 => "internal error",
        RobotStuck2 = 90 => "robot stuck 2",
        RobotStuckOnTables = 91 => "robot stuck on tables",
        RobotStuckOnPassage = 92 => "robot stuck on passage",
        RobotStuckOnThreshold = 93 => "robot stuck on threshold",
        RobotStuckOnLowLyingArea = 94 => "robot stuck on low lying area",
        RobotStuckOnRamp = 95 => "robot stuck on ramp",
        RobotStuckOnObstacle = 96 => "robot stuck on obstacle",
        RobotStuckOnPet = 97 => "robot stuck on pet",
        RobotStuckOnSlipperySurface = 98 => "robot stuck on slippery surface",
        RobotStuckOnCarpet = 99 => "robot stuck on carpet",
        BinFull = 101 => "bin full",
        BinOpen = 102 => "bin open",
        BinOpen2 = 103 => "bin open 2",
        BinFull2 = 104 => "bin full 2",
        WaterTank = 105 => "water tank",
        DirtyWaterTank = 106 => "dirty water tank",
        WaterTankDry = 107 => "water tank dry",
        DirtyWaterTank2 = 108 => "dirty water tank 2",
        DirtyWaterTankBlocked = 109 => "dirty water tank blocked",
        DirtyWaterTankPump = 110 => "dirty water tank pump",
        MopPad = 111 => "mop pad",
        WetMopPad = 112 => "wet mop pad",
        CleanMopPad = 114 => "clean mop pad",
        CleanTankLevel = 116 => "clean tank level",
        StationDisconnected = 117 => "station disconnected",
        DirtyTankLevel = 118 => "dirty tank level",
        WashboardLevel = 119 => "washboard level",
        NoMopInStation = 120 => "no mop in station",
        DustBagFull = 121 => "dust bag full",
        UnknownWarning = 122 => "unknown warning",
        SelfTestFailed = 123 => "self test failed",
        WashboardNotWorking = 124 => "washboard not working",
        DrainageFailed = 125 => "drainage failed",
        MopNotDetected = 126 => "mop not detected",
        MopHolderError = 127 => "mop holder error",
        DockError = 128 => "dock error",
        WashFailed = 129 => "wash failed",
        RobotStuckOnCurtain = 200 => "robot stuck on curtain",
        EdgeMopStopRotate = 201 => "edge mop stop rotate",
        EdgeMopDetached = 202 => "edge mop detached",
        ChassisLiftMalfunction = 203 => "chassis lift malfunction",
        InternalError2 = 207 => "internal error 2",
        MopCoverError = 209 => "mop cover error",
        RollerMopError = 210 => "roller mop error",
        RoboticArmStopped = 212 => "robotic arm stopped",
        OnboardWaterTankEmpty = 213 => "onboard water tank empty",
        OnboardDirtyWaterTankFull = 214 => "onboard dirty water tank full",
        MopNotInstalled = 215 => "mop not installed",
        LdsError2 = 217 => "lds error 2",
        RollerMopError2 = 218 => "roller mop error 2",
        FluffingRollerError = 222 => "fluffing roller error",
        MopCoverError2 = 223 => "mop cover error 2",
        MopCoverError3 = 224 => "mop cover error 3",
        RollerMopError3 = 225 => "roller mop error 3",
        BlockedByObstacle = 226 => "blocked by obstacle",
        DrainageOutletFilter = 227 => "drainage outlet filter",
        MainWheelsError = 228 => "main wheels error",
        InternalError3 = 229 => "internal error 3",
        InternalError4 = 230 => "internal error 4",
        ReturnToChargeFailed = 1000 => "return to charge failed",
    }
}

open_enum! {
    /// Charging status, property 3.2.
    ChargingStatus {
        Charging = 1 => "charging",
        NotCharging = 2 => "not charging",
        ChargingCompleted = 3 => "charging completed",
        ReturnToCharge = 5 => "return to charge",
    }
}

open_enum! {
    /// Suction level, property 4.4.
    SuctionLevel {
        Quiet = 0 => "quiet",
        Standard = 1 => "standard",
        Strong = 2 => "strong",
        Turbo = 3 => "turbo",
    }
}

open_enum! {
    /// Logical cleaning mode. The raw value in 4.23 needs a polarity; see `codec::cleaning_mode`.
    CleaningMode {
        Sweeping = 0 => "sweeping",
        Mopping = 1 => "mopping",
        SweepingAndMopping = 2 => "sweeping and mopping",
        MoppingAfterSweeping = 3 => "mopping after sweeping",
    }
}

open_enum! {
    /// Water tank or mop state, property 4.6.
    WaterTank {
        NotInstalled = 0 => "not installed",
        Installed = 1 => "installed",
        MopInstalled = 10 => "mop installed",
        InStation = 99 => "in station",
    }
}

open_enum! {
    /// Water volume, property 4.5 (models without a self-wash base).
    WaterVolume {
        Low = 1 => "low",
        Medium = 2 => "medium",
        High = 3 => "high",
    }
}

open_enum! {
    /// Mop pad humidity, 4.23 high byte on models without a wetness level.
    MopPadHumidity {
        SlightlyDry = 1 => "slightly dry",
        Moist = 2 => "moist",
        Wet = 3 => "wet",
    }
}

open_enum! {
    /// Carpet sensitivity, property 4.28.
    CarpetSensitivity {
        Low = 1 => "low",
        Medium = 2 => "medium",
        High = 3 => "high",
    }
}

open_enum! {
    /// Carpet cleaning behaviour, property 4.36.
    CarpetCleaning {
        NotSet = 0 => "not set",
        Avoidance = 1 => "avoidance",
        Adaptation = 2 => "adaptation",
        RemoveMop = 3 => "remove mop",
        AdaptationWithoutRoute = 4 => "adaptation without route",
        VacuumAndMop = 5 => "vacuum and mop",
        Ignore = 6 => "ignore",
        Cross = 7 => "cross",
    }
}

open_enum! {
    /// Relocation status, property 4.20.
    RelocationStatus {
        Located = 0 => "located",
        Locating = 1 => "locating",
        Failed = 10 => "failed",
        Success = 11 => "success",
    }
}

open_enum! {
    /// Dust collection availability, property 15.3.
    DustCollection {
        NotAvailable = 0 => "not available",
        Available = 1 => "available",
        OverUse = 2 => "over use",
        Never = 3 => "never",
    }
}

open_enum! {
    /// Auto-empty status, property 15.5.
    AutoEmptyStatus {
        Idle = 0 => "idle",
        Active = 1 => "active",
        NotPerformed = 2 => "not performed",
        Returning = 3 => "returning",
    }
}

open_enum! {
    /// Self-wash base status, property 4.25.
    SelfWashBaseStatus {
        Idle = 0 => "idle",
        Washing = 1 => "washing",
        Drying = 2 => "drying",
        Returning = 3 => "returning",
        Paused = 4 => "paused",
        CleanAddWater = 5 => "clean add water",
        ReturningToDrain = 6 => "returning to drain",
        ReturningForDryMop = 7 => "returning for dry mop",
    }
}

open_enum! {
    /// Mop wash level, property 4.46.
    MopWashLevel {
        WaterSaving = 0 => "water saving",
        Daily = 1 => "daily",
        Deep = 2 => "deep",
    }
}

open_enum! {
    /// Mopping type, settings key `CleanType` in 4.50.
    MoppingType {
        Daily = 0 => "daily",
        Accurate = 1 => "accurate",
        Deep = 2 => "deep",
    }
}

open_enum! {
    /// Cleaning route, settings key `CleanRoute` in 4.50.
    CleaningRoute {
        NotSet = 0 => "not set",
        Standard = 1 => "standard",
        Intensive = 2 => "intensive",
        Deep = 3 => "deep",
        Quick = 4 => "quick",
    }
}

open_enum! {
    /// Self-clean frequency, settings key `BackWashType` in 4.50.
    SelfCleanFrequency {
        ByArea = 1 => "by area",
        ByTime = 2 => "by time",
        ByRoom = 3 => "by room",
        Intelligent = 4 => "intelligent",
    }
}

open_enum! {
    /// Mop extend frequency, settings key `ExtrFreq` in 4.50.
    MopExtendFrequency {
        Standard = 7 => "standard",
        Intelligent = 1 => "intelligent",
        High = 2 => "high",
    }
}

open_enum! {
    /// Auto-empty mode 15.1 on models without `INTELLIGENT_AUTO_EMPTY`.
    AutoEmptyMode {
        Off = 0 => "off",
        Standard = 1 => "standard",
        HighFrequency = 2 => "high frequency",
        LowFrequency = 3 => "low frequency",
    }
}

open_enum! {
    /// Auto-empty mode 15.1 on models with `INTELLIGENT_AUTO_EMPTY`.
    AutoEmptyModeV2 {
        Off = 0 => "off",
        Standard = 1 => "standard",
        Intelligent = 2 => "intelligent",
        LowFrequency = 3 => "low frequency",
        CustomFrequency = 4 => "custom frequency",
        HighFrequency = 5 => "high frequency",
    }
}

open_enum! {
    /// CleanGenius, settings key `SmartHost` in 4.50.
    CleanGenius {
        Off = 0 => "off",
        RoutineCleaning = 1 => "routine cleaning",
        DeepCleaning = 2 => "deep cleaning",
    }
}

open_enum! {
    /// CleanGenius mode, property 28.5.
    CleanGeniusMode {
        VacuumAndMop = 2 => "vacuum and mop",
        MopAfterVacuum = 3 => "mop after vacuum",
    }
}

open_enum! {
    /// Second cleaning, settings keys `SmartAutoMop` and `SmartAutoWash` in 4.50.
    SecondCleaning {
        Off = 0 => "off",
        InDeepMode = 1 => "in deep mode",
        InAllModes = 2 => "in all modes",
    }
}

open_enum! {
    /// Floor material of a map room.
    FloorMaterial {
        None = 0 => "none",
        Wood = 1 => "wood",
        Tile = 2 => "tile",
        MediumPileCarpet = 5 => "medium pile carpet",
        LowPileCarpet = 6 => "low pile carpet",
        Carpet = 7 => "carpet",
    }
}

open_enum! {
    /// Low water warning, property 4.41.
    LowWaterWarning {
        NoWarning = 0 => "no warning",
        NoWaterLeftDismiss = 1 => "no water left dismiss",
        NoWaterLeft = 2 => "no water left",
        NoWaterLeftAfterClean = 3 => "no water left after clean",
        NoWaterForClean = 4 => "no water for clean",
        LowWater = 5 => "low water",
        TankNotInstalled = 6 => "tank not installed",
    }
}

open_enum! {
    /// Drainage status, property 4.60.
    DrainageStatus {
        Idle = 0 => "idle",
        Draining = 1 => "draining",
        DrainingSuccess = 2 => "draining success",
        DrainingFailed = 3 => "draining failed",
    }
}

open_enum! {
    /// Map recovery status, property 6.11.
    MapRecoveryStatus {
        Idle = 0 => "idle",
        Running = 2 => "running",
        Success = 3 => "success",
        Fail = 4 => "fail",
        Fail2 = 5 => "fail 2",
    }
}

open_enum! {
    /// Map backup status, property 6.14.
    MapBackupStatus {
        Idle = 0 => "idle",
        Running = 2 => "running",
        Success = 3 => "success",
        Fail = 4 => "fail",
    }
}

open_enum! {
    /// Station clean water tank, property 27.1.
    CleanWaterTankStatus {
        Installed = 0 => "installed",
        NotInstalled = 1 => "not installed",
        LowWater = 2 => "low water",
        Checking = 3 => "checking",
    }
}

open_enum! {
    /// Station dirty water tank, property 27.2.
    DirtyWaterTankStatus {
        Installed = 0 => "installed",
        NotInstalledOrFull = 1 => "not installed or full",
    }
}

open_enum! {
    /// Station dust bag, property 27.3.
    DustBagStatus {
        Installed = 0 => "installed",
        NotInstalled = 1 => "not installed",
        Check = 2 => "check",
    }
}

open_enum! {
    /// Station detergent, property 27.4.
    DetergentStatus {
        Installed = 0 => "installed",
        Disabled = 1 => "disabled",
        LowDetergent = 2 => "low detergent",
    }
}

open_enum! {
    /// Hot water, property 27.15.
    HotWaterStatus {
        Disabled = 0 => "disabled",
        Enabled = 1 => "enabled",
    }
}

open_enum! {
    /// Station drainage, property 27.5.
    StationDrainageStatus {
        Idle = 0 => "idle",
        Draining = 1 => "draining",
    }
}

open_enum! {
    /// Dust bag drying, property 27.18.
    DustBagDryingStatus {
        Idle = 0 => "idle",
        Drying = 1 => "drying",
        Paused = 2 => "paused",
    }
}

open_enum! {
    /// Water temperature, property 28.8.
    WaterTemperature {
        Normal = 0 => "normal",
        Mild = 1 => "mild",
        Warm = 2 => "warm",
        Hot = 3 => "hot",
        Max = 4 => "max",
    }
}

open_enum! {
    /// Result of a cleaning job, property 4.13 and event 4.e1.
    CleanLogStatus {
        Interrupted = 0 => "interrupted",
        Completed = 1 => "completed",
        ManuallyEnded = 2 => "manually ended",
        Failed = 3 => "failed",
    }
}

