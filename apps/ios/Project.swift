import ProjectDescription

// ThinClaw Apple platform project: four thin target shells over the local
// SPM packages in Packages/ (where all real code lives). Signing is xcconfig
// driven — copy Config/Signing.example.xcconfig to Config/Signing.local.xcconfig
// and set your team; nothing secret is committed.

let baseSettings: Settings = .settings(
    configurations: [
        .debug(name: "Debug", xcconfig: "Config/Base.xcconfig"),
        .release(name: "Release", xcconfig: "Config/Base.xcconfig"),
    ]
)

let project = Project(
    name: "ThinClaw",
    organizationName: "ThinClaw",
    options: .options(
        automaticSchemesOptions: .disabled,
        disableBundleAccessors: true,
        disableSynthesizedResourceAccessors: true
    ),
    packages: [
        .package(path: "Packages/ThinClawAPI"),
        .package(path: "Packages/ThinClawTransport"),
        .package(path: "Packages/ThinClawCore"),
        .package(path: "Packages/ThinClawPersistence"),
        .package(path: "Packages/ThinClawAuth"),
        .package(path: "Packages/ThinClawSnapshotKit"),
        .package(path: "Packages/ThinClawDesign"),
        .package(path: "Packages/ThinClawWidgetKitShared"),
        .package(path: "Packages/ThinClawWatchBridge"),
        .package(path: "Packages/Features/FeatureOnboarding"),
        .package(path: "Packages/Features/FeatureChat"),
        .package(path: "Packages/Features/FeatureSessions"),
        .package(path: "Packages/Features/FeatureApprovals"),
        .package(path: "Packages/Features/FeatureJobs"),
        .package(path: "Packages/Features/FeatureSettings"),
    ],
    settings: baseSettings,
    targets: [
        .target(
            name: "ThinClaw",
            destinations: [.iPhone, .iPad],
            product: .app,
            bundleId: "com.thinclaw.ios",
            deploymentTargets: .iOS("26.0"),
            infoPlist: .extendingDefault(with: [
                "UILaunchScreen": ["UIColorName": "", "UIImageName": ""],
                "NSSupportsLiveActivities": true,
                "NSLocalNetworkUsageDescription":
                    "ThinClaw discovers and connects to your self-hosted gateway on the local network.",
                "NSBonjourServices": ["_thinclaw._tcp"],
                "NSCameraUsageDescription":
                    "The camera scans the pairing QR code shown by your gateway.",
                "CFBundleURLTypes": [
                    [
                        "CFBundleURLName": "com.thinclaw.ios",
                        "CFBundleURLSchemes": ["thinclaw"],
                    ]
                ],
                "BGTaskSchedulerPermittedIdentifiers": ["com.thinclaw.ios.refresh"],
                "UIBackgroundModes": ["remote-notification", "processing"],
            ]),
            sources: ["App/Sources/**"],
            resources: ["App/Resources/**"],
            entitlements: "App/ThinClaw.entitlements",
            dependencies: [
                .target(name: "ThinClawWidgets"),
                .target(name: "ThinClawWatch"),
                .target(name: "ThinClawNotificationService"),
                .package(product: "ThinClawAPI"),
                .package(product: "ThinClawTransport"),
                .package(product: "ThinClawCore"),
                .package(product: "ThinClawPersistence"),
                .package(product: "ThinClawAuth"),
                .package(product: "ThinClawSnapshotKit"),
                .package(product: "ThinClawDesign"),
                .package(product: "ThinClawWidgetKitShared"),
                .package(product: "ThinClawWatchBridge"),
                .package(product: "FeatureOnboarding"),
                .package(product: "FeatureChat"),
                .package(product: "FeatureSessions"),
                .package(product: "FeatureApprovals"),
                .package(product: "FeatureJobs"),
                .package(product: "FeatureSettings"),
            ],
            settings: .settings(configurations: [
                .debug(name: "Debug", xcconfig: "Config/App.xcconfig"),
                .release(name: "Release", xcconfig: "Config/App.xcconfig"),
            ])
        ),
        .target(
            name: "ThinClawWidgets",
            destinations: [.iPhone, .iPad],
            product: .appExtension,
            bundleId: "com.thinclaw.ios.widgets",
            deploymentTargets: .iOS("26.0"),
            infoPlist: .extendingDefault(with: [
                "NSExtension": [
                    "NSExtensionPointIdentifier": "com.apple.widgetkit-extension"
                ]
            ]),
            sources: ["Widgets/Sources/**"],
            entitlements: "Widgets/Widgets.entitlements",
            dependencies: [
                .package(product: "ThinClawSnapshotKit"),
                .package(product: "ThinClawAuth"),
                .package(product: "ThinClawAPI"),
                .package(product: "ThinClawDesign"),
                .package(product: "ThinClawWidgetKitShared"),
            ],
            settings: .settings(configurations: [
                .debug(name: "Debug", xcconfig: "Config/Widgets.xcconfig"),
                .release(name: "Release", xcconfig: "Config/Widgets.xcconfig"),
            ])
        ),
        .target(
            name: "ThinClawNotificationService",
            destinations: [.iPhone, .iPad],
            product: .appExtension,
            bundleId: "com.thinclaw.ios.notificationservice",
            deploymentTargets: .iOS("26.0"),
            infoPlist: .extendingDefault(with: [
                "NSExtension": [
                    "NSExtensionPointIdentifier": "com.apple.usernotifications.service",
                    "NSExtensionPrincipalClass":
                        "$(PRODUCT_MODULE_NAME).NotificationService",
                ]
            ]),
            sources: ["NotificationService/Sources/**"],
            entitlements: "NotificationService/NotificationService.entitlements",
            dependencies: [
                .package(product: "ThinClawAuth"),
                .package(product: "ThinClawAPI"),
            ],
            settings: .settings(configurations: [
                .debug(name: "Debug", xcconfig: "Config/NotificationService.xcconfig"),
                .release(name: "Release", xcconfig: "Config/NotificationService.xcconfig"),
            ])
        ),
        .target(
            name: "ThinClawWatch",
            destinations: [.appleWatch],
            product: .app,
            bundleId: "com.thinclaw.ios.watchkitapp",
            deploymentTargets: .watchOS("26.0"),
            infoPlist: .extendingDefault(with: [
                "WKApplication": true,
                "WKCompanionAppBundleIdentifier": "com.thinclaw.ios",
            ]),
            sources: ["Watch/Sources/**"],
            entitlements: "Watch/Watch.entitlements",
            dependencies: [
                .target(name: "ThinClawWatchWidgets"),
                .package(product: "ThinClawCore"),
                .package(product: "ThinClawSnapshotKit"),
                .package(product: "ThinClawDesign"),
                .package(product: "ThinClawWatchBridge"),
            ],
            settings: .settings(configurations: [
                .debug(name: "Debug", xcconfig: "Config/Watch.xcconfig"),
                .release(name: "Release", xcconfig: "Config/Watch.xcconfig"),
            ])
        ),
        .target(
            name: "ThinClawWatchWidgets",
            destinations: [.appleWatch],
            product: .appExtension,
            bundleId: "com.thinclaw.ios.watchkitapp.widgets",
            deploymentTargets: .watchOS("26.0"),
            infoPlist: .extendingDefault(with: [
                "NSExtension": [
                    "NSExtensionPointIdentifier": "com.apple.widgetkit-extension"
                ]
            ]),
            sources: ["WatchWidgets/Sources/**"],
            entitlements: "WatchWidgets/WatchWidgets.entitlements",
            dependencies: [
                .package(product: "ThinClawSnapshotKit")
            ],
            settings: .settings(configurations: [
                .debug(name: "Debug", xcconfig: "Config/WatchWidgets.xcconfig"),
                .release(name: "Release", xcconfig: "Config/WatchWidgets.xcconfig"),
            ])
        ),
    ],
    schemes: [
        .scheme(
            name: "ThinClaw",
            shared: true,
            buildAction: .buildAction(targets: ["ThinClaw"]),
            runAction: .runAction(executable: "ThinClaw")
        ),
        .scheme(
            name: "ThinClawWatch",
            shared: true,
            buildAction: .buildAction(targets: ["ThinClawWatch"]),
            runAction: .runAction(executable: "ThinClawWatch")
        ),
    ]
)
