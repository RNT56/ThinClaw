import ProjectDescription

// ThinClaw Apple platform project: four thin target shells over the local
// SPM packages in Packages/ (where all real code lives). Signing is xcconfig
// driven — copy Config/Signing.example.xcconfig to Config/Signing.local.xcconfig
// and set your team; nothing secret is committed.

let swift6Settings: SettingsDictionary = [
    "SWIFT_VERSION": "6.0",
    "SWIFT_STRICT_CONCURRENCY": "complete",
    "SWIFT_TREAT_WARNINGS_AS_ERRORS": "YES",
]

let baseSettings: Settings = .settings(
    base: swift6Settings,
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
        // These are also dependencies of ThinClawAPI. Declaring them at the
        // project boundary lets executable/extension targets link the runtime
        // products explicitly when they consume generated API types through a
        // local package (notably the watch direct-fallback client).
        .package(
            url: "https://github.com/apple/swift-openapi-runtime",
            from: "1.8.0"
        ),
        .package(
            url: "https://github.com/apple/swift-openapi-urlsession",
            from: "1.1.0"
        ),
        .package(
            url: "https://github.com/apple/swift-http-types",
            from: "1.4.0"
        ),
        .package(path: "Packages/ThinClawAPI"),
        .package(path: "Packages/ThinClawTransport"),
        .package(path: "Packages/ThinClawCore"),
        .package(path: "Packages/ThinClawPersistence"),
        .package(path: "Packages/ThinClawAuth"),
        .package(path: "Packages/ThinClawSnapshotKit"),
        .package(path: "Packages/ThinClawLiveActivity"),
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
            deploymentTargets: .iOS("18.0"),
            infoPlist: .extendingDefault(with: [
                "CFBundleShortVersionString": "$(MARKETING_VERSION)",
                "CFBundleVersion": "$(CURRENT_PROJECT_VERSION)",
                "UILaunchScreen": [
                    "UIColorName": "LaunchBackground",
                    "UIImageName": "LaunchMark",
                ],
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
                "UIBackgroundModes": ["remote-notification", "fetch"],
            ]),
            sources: ["App/Sources/**"],
            resources: ["App/Resources/**", "Shared/PrivacyInfo.xcprivacy"],
            entitlements: "App/ThinClaw.entitlements",
            dependencies: [
                .target(name: "ThinClawWidgets"),
                .target(name: "ThinClawWatch"),
                .target(name: "ThinClawNotificationService"),
                .package(product: "ThinClawAPI"),
                .package(product: "OpenAPIRuntime"),
                .package(product: "OpenAPIURLSession"),
                .package(product: "HTTPTypes"),
                .package(product: "ThinClawTransport"),
                .package(product: "ThinClawCore"),
                .package(product: "ThinClawPersistence"),
                .package(product: "ThinClawAuth"),
                .package(product: "ThinClawSnapshotKit"),
                .package(product: "ThinClawLiveActivity"),
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
            settings: .settings(
                base: swift6Settings,
                configurations: [
                    .debug(name: "Debug", xcconfig: "Config/App.xcconfig"),
                    .release(name: "Release", xcconfig: "Config/App.xcconfig"),
                ])
        ),
        .target(
            name: "ThinClawWidgets",
            destinations: [.iPhone, .iPad],
            product: .appExtension,
            bundleId: "com.thinclaw.ios.widgets",
            deploymentTargets: .iOS("18.0"),
            infoPlist: .extendingDefault(with: [
                "CFBundleShortVersionString": "$(MARKETING_VERSION)",
                "CFBundleVersion": "$(CURRENT_PROJECT_VERSION)",
                "NSExtension": [
                    "NSExtensionPointIdentifier": "com.apple.widgetkit-extension"
                ],
            ]),
            sources: ["Widgets/Sources/**"],
            resources: ["Shared/PrivacyInfo.xcprivacy"],
            entitlements: "Widgets/Widgets.entitlements",
            dependencies: [
                .package(product: "ThinClawSnapshotKit"),
                .package(product: "ThinClawAuth"),
                .package(product: "ThinClawAPI"),
                .package(product: "OpenAPIRuntime"),
                .package(product: "OpenAPIURLSession"),
                .package(product: "HTTPTypes"),
                .package(product: "ThinClawDesign"),
                .package(product: "ThinClawWidgetKitShared"),
            ],
            settings: .settings(
                base: swift6Settings,
                configurations: [
                    .debug(name: "Debug", xcconfig: "Config/Widgets.xcconfig"),
                    .release(name: "Release", xcconfig: "Config/Widgets.xcconfig"),
                ])
        ),
        .target(
            name: "ThinClawNotificationService",
            destinations: [.iPhone, .iPad],
            product: .appExtension,
            bundleId: "com.thinclaw.ios.notificationservice",
            deploymentTargets: .iOS("18.0"),
            infoPlist: .extendingDefault(with: [
                "CFBundleShortVersionString": "$(MARKETING_VERSION)",
                "CFBundleVersion": "$(CURRENT_PROJECT_VERSION)",
                "NSExtension": [
                    "NSExtensionPointIdentifier": "com.apple.usernotifications.service",
                    "NSExtensionPrincipalClass":
                        "$(PRODUCT_MODULE_NAME).NotificationService",
                ],
            ]),
            sources: ["NotificationService/Sources/**"],
            resources: ["Shared/PrivacyInfo.xcprivacy"],
            entitlements: "NotificationService/NotificationService.entitlements",
            dependencies: [
                .package(product: "ThinClawAuth"),
                .package(product: "ThinClawAPI"),
                .package(product: "OpenAPIRuntime"),
                .package(product: "OpenAPIURLSession"),
                .package(product: "HTTPTypes"),
            ],
            settings: .settings(
                base: swift6Settings,
                configurations: [
                    .debug(name: "Debug", xcconfig: "Config/NotificationService.xcconfig"),
                    .release(name: "Release", xcconfig: "Config/NotificationService.xcconfig"),
                ])
        ),
        .target(
            name: "ThinClawWatch",
            destinations: [.appleWatch],
            product: .app,
            bundleId: "com.thinclaw.ios.watchkitapp",
            deploymentTargets: .watchOS("11.0"),
            infoPlist: .extendingDefault(with: [
                "CFBundleShortVersionString": "$(MARKETING_VERSION)",
                "CFBundleVersion": "$(CURRENT_PROJECT_VERSION)",
                "WKApplication": true,
                "WKCompanionAppBundleIdentifier": "com.thinclaw.ios",
            ]),
            sources: ["Watch/Sources/**"],
            resources: ["Shared/PrivacyInfo.xcprivacy"],
            entitlements: "Watch/Watch.entitlements",
            dependencies: [
                .target(name: "ThinClawWatchWidgets"),
                .package(product: "ThinClawCore"),
                .package(product: "ThinClawSnapshotKit"),
                .package(product: "ThinClawDesign"),
                // WatchBridge's direct-gateway fallback exposes generated API
                // types. Link the product at the executable boundary so SPM
                // also embeds OpenAPIRuntime/URLSession on watchOS.
                .package(product: "ThinClawAPI"),
                .package(product: "OpenAPIRuntime"),
                .package(product: "OpenAPIURLSession"),
                .package(product: "HTTPTypes"),
                .package(product: "ThinClawWatchBridge"),
            ],
            settings: .settings(
                base: swift6Settings,
                configurations: [
                    .debug(name: "Debug", xcconfig: "Config/Watch.xcconfig"),
                    .release(name: "Release", xcconfig: "Config/Watch.xcconfig"),
                ])
        ),
        .target(
            name: "ThinClawWatchWidgets",
            destinations: [.appleWatch],
            product: .appExtension,
            bundleId: "com.thinclaw.ios.watchkitapp.widgets",
            deploymentTargets: .watchOS("11.0"),
            infoPlist: .extendingDefault(with: [
                "CFBundleShortVersionString": "$(MARKETING_VERSION)",
                "CFBundleVersion": "$(CURRENT_PROJECT_VERSION)",
                "NSExtension": [
                    "NSExtensionPointIdentifier": "com.apple.widgetkit-extension"
                ],
            ]),
            sources: ["WatchWidgets/Sources/**"],
            resources: ["Shared/PrivacyInfo.xcprivacy"],
            entitlements: "WatchWidgets/WatchWidgets.entitlements",
            dependencies: [
                .package(product: "ThinClawSnapshotKit")
            ],
            settings: .settings(
                base: swift6Settings,
                configurations: [
                    .debug(name: "Debug", xcconfig: "Config/WatchWidgets.xcconfig"),
                    .release(name: "Release", xcconfig: "Config/WatchWidgets.xcconfig"),
                ])
        ),
        .target(
            name: "ThinClawTests",
            destinations: [.iPhone, .iPad],
            product: .unitTests,
            bundleId: "com.thinclaw.ios.tests",
            deploymentTargets: .iOS("18.0"),
            infoPlist: .default,
            sources: ["App/Tests/**"],
            dependencies: [
                .target(name: "ThinClaw"),
                .package(product: "ThinClawWidgetKitShared"),
            ],
            settings: .settings(
                base: swift6Settings,
                configurations: [
                    .debug(name: "Debug", xcconfig: "Config/Base.xcconfig"),
                    .release(name: "Release", xcconfig: "Config/Base.xcconfig"),
                ])
        ),
        .target(
            name: "ThinClawUITests",
            destinations: [.iPhone, .iPad],
            product: .uiTests,
            bundleId: "com.thinclaw.ios.uitests",
            deploymentTargets: .iOS("18.0"),
            infoPlist: .default,
            sources: ["App/UITests/**"],
            dependencies: [.target(name: "ThinClaw")],
            settings: .settings(
                base: swift6Settings,
                configurations: [
                    .debug(name: "Debug", xcconfig: "Config/Base.xcconfig"),
                    .release(name: "Release", xcconfig: "Config/Base.xcconfig"),
                ])
        ),
        .target(
            name: "ThinClawNotificationServiceTests",
            destinations: [.iPhone, .iPad],
            product: .unitTests,
            bundleId: "com.thinclaw.ios.notificationservice.tests",
            deploymentTargets: .iOS("18.0"),
            infoPlist: .default,
            sources: [
                "NotificationService/Sources/**",
                "NotificationService/Tests/**",
            ],
            dependencies: [
                .package(product: "ThinClawAuth"),
                .package(product: "ThinClawAPI"),
                .package(product: "OpenAPIRuntime"),
                .package(product: "OpenAPIURLSession"),
                .package(product: "HTTPTypes"),
            ],
            settings: .settings(
                base: swift6Settings,
                configurations: [
                    .debug(name: "Debug", xcconfig: "Config/Base.xcconfig"),
                    .release(name: "Release", xcconfig: "Config/Base.xcconfig"),
                ])
        ),
    ],
    schemes: [
        .scheme(
            name: "ThinClaw",
            shared: true,
            buildAction: .buildAction(targets: ["ThinClaw"]),
            testAction: .targets([
                "ThinClawTests",
                "ThinClawNotificationServiceTests",
                "ThinClawUITests",
            ]),
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
