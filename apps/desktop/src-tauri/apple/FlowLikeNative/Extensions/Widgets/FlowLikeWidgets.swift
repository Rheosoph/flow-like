import AppIntents
import FlowLikeNative
import SwiftUI
import WidgetKit
#if os(iOS)
import ActivityKit
import UIKit
#else
import AppKit
struct FlowLikeWidgetIntentPackage: AppIntentsPackage {
    static var includedPackages: [any AppIntentsPackage.Type] { [FlowLikeNativePackage.self] }
}
#endif

struct WorkspaceEntry: TimelineEntry {
    let date: Date
    let snapshot: NativeSnapshot?
}

struct WorkspaceProvider: TimelineProvider {
    func placeholder(in context: Context) -> WorkspaceEntry { WorkspaceEntry(date: Date(), snapshot: nil) }
    func getSnapshot(in context: Context, completion: @escaping (WorkspaceEntry) -> Void) {
        completion(WorkspaceEntry(date: Date(), snapshot: NativeStore.shared.snapshot()))
    }
    func getTimeline(in context: Context, completion: @escaping (Timeline<WorkspaceEntry>) -> Void) {
        let snapshot = NativeStore.shared.snapshot()
        let now = Date()
        var entries = [WorkspaceEntry(date: now, snapshot: snapshot)]
        if let expiry = snapshot.flatMap({ NativeSnapshot.date($0.expiresAt) }), expiry > now {
            entries.append(WorkspaceEntry(date: expiry, snapshot: nil))
        }
        completion(Timeline(entries: entries, policy: .after(now.addingTimeInterval(900))))
    }
}

// The mark follows the two paths in gen/apple/icon.icon/Assets/flow-like.svg.
struct FlowLikeMark: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: 391.948, y: 793.118))
        path.addCurve(to: CGPoint(x: 418.948, y: 791.618), control1: CGPoint(x: 402.463, y: 792.135), control2: CGPoint(x: 408.487, y: 793.06))
        path.addLine(to: CGPoint(x: 419.399, y: 791.555))
        path.addCurve(to: CGPoint(x: 430.448, y: 789.618), control1: CGPoint(x: 423.638, y: 790.971), control2: CGPoint(x: 426.16, y: 790.624))
        path.addCurve(to: CGPoint(x: 443.448, y: 785.618), control1: CGPoint(x: 435.619, y: 788.404), control2: CGPoint(x: 438.424, y: 787.341))
        path.addCurve(to: CGPoint(x: 458.948, y: 779.618), control1: CGPoint(x: 449.588, y: 783.512), control2: CGPoint(x: 458.948, y: 779.618))
        path.addCurve(to: CGPoint(x: 478.448, y: 769.618), control1: CGPoint(x: 458.948, y: 779.618), control2: CGPoint(x: 471.387, y: 774.088))
        path.addCurve(to: CGPoint(x: 495.448, y: 756.618), control1: CGPoint(x: 485.509, y: 765.147), control2: CGPoint(x: 489.026, y: 761.966))
        path.addCurve(to: CGPoint(x: 511.448, y: 742.118), control1: CGPoint(x: 501.928, y: 751.222), control2: CGPoint(x: 505.849, y: 748.423))
        path.addCurve(to: CGPoint(x: 524.448, y: 723.618), control1: CGPoint(x: 517.311, y: 735.515), control2: CGPoint(x: 519.772, y: 731.108))
        path.addCurve(to: CGPoint(x: 532.448, y: 709.618), control1: CGPoint(x: 527.782, y: 718.276), control2: CGPoint(x: 529.587, y: 715.227))
        path.addCurve(to: CGPoint(x: 541.948, y: 687.118), control1: CGPoint(x: 536.781, y: 701.121), control2: CGPoint(x: 538.888, y: 696.151))
        path.addCurve(to: CGPoint(x: 546.948, y: 669.118), control1: CGPoint(x: 544.289, y: 680.208), control2: CGPoint(x: 545.385, y: 676.244))
        path.addCurve(to: CGPoint(x: 549.948, y: 650.618), control1: CGPoint(x: 548.516, y: 661.969), control2: CGPoint(x: 549.046, y: 657.881))
        path.addCurve(to: CGPoint(x: 551.448, y: 634.118), control1: CGPoint(x: 550.745, y: 644.197), control2: CGPoint(x: 550.584, y: 640.53))
        path.addCurve(to: CGPoint(x: 556.948, y: 607.618), control1: CGPoint(x: 552.859, y: 623.643), control2: CGPoint(x: 553.687, y: 617.671))
        path.addCurve(to: CGPoint(x: 569.448, y: 580.618), control1: CGPoint(x: 560.533, y: 596.565), control2: CGPoint(x: 563.505, y: 590.602))
        path.addCurve(to: CGPoint(x: 582.948, y: 561.618), control1: CGPoint(x: 574.104, y: 572.796), control2: CGPoint(x: 577.023, y: 568.527))
        path.addCurve(to: CGPoint(x: 606.5, y: 538.5), control1: CGPoint(x: 590.866, y: 552.385), control2: CGPoint(x: 599.527, y: 543.5))
        path.addCurve(to: CGPoint(x: 634.0, y: 523.5), control1: CGPoint(x: 613.473, y: 533.5), control2: CGPoint(x: 624.5, y: 527.297))
        path.addCurve(to: CGPoint(x: 660.448, y: 516.118), control1: CGPoint(x: 643.5, y: 519.703), control2: CGPoint(x: 649.5, y: 518.17))
        path.addCurve(to: CGPoint(x: 679.948, y: 514.118), control1: CGPoint(x: 668.448, y: 514.618), control2: CGPoint(x: 674.448, y: 514.118))
        path.addCurve(to: CGPoint(x: 730.809, y: 514.118), control1: CGPoint(x: 698.445, y: 514.118), control2: CGPoint(x: 714.193, y: 514.118))
        path.addLine(to: CGPoint(x: 757.948, y: 514.118))
        path.addCurve(to: CGPoint(x: 804.448, y: 516.118), control1: CGPoint(x: 776.303, y: 514.118), control2: CGPoint(x: 787.396, y: 512.874))
        path.addLine(to: CGPoint(x: 805.631, y: 516.343))
        path.addCurve(to: CGPoint(x: 836.0, y: 526.0), control1: CGPoint(x: 821.603, y: 519.379), control2: CGPoint(x: 823.846, y: 519.805))
        path.addCurve(to: CGPoint(x: 855.5, y: 538.5), control1: CGPoint(x: 848.448, y: 532.345), control2: CGPoint(x: 847.979, y: 532.383))
        path.addCurve(to: CGPoint(x: 875.948, y: 561.118), control1: CGPoint(x: 864.712, y: 545.993), control2: CGPoint(x: 869.072, y: 551.437))
        path.addCurve(to: CGPoint(x: 887.948, y: 584.618), control1: CGPoint(x: 881.915, y: 569.519), control2: CGPoint(x: 884.36, y: 574.958))
        path.addCurve(to: CGPoint(x: 893.448, y: 605.118), control1: CGPoint(x: 890.835, y: 592.388), control2: CGPoint(x: 892.023, y: 596.952))
        path.addCurve(to: CGPoint(x: 894.948, y: 633.618), control1: CGPoint(x: 895.364, y: 616.097), control2: CGPoint(x: 895.776, y: 622.503))
        path.addCurve(to: CGPoint(x: 890.948, y: 656.118), control1: CGPoint(x: 894.285, y: 642.518), control2: CGPoint(x: 893.179, y: 647.477))
        path.addCurve(to: CGPoint(x: 884.448, y: 675.618), control1: CGPoint(x: 888.941, y: 663.89), control2: CGPoint(x: 887.63, y: 668.248))
        path.addCurve(to: CGPoint(x: 873.448, y: 695.618), control1: CGPoint(x: 880.915, y: 683.801), control2: CGPoint(x: 878.527, y: 688.292))
        path.addCurve(to: CGPoint(x: 861.448, y: 710.118), control1: CGPoint(x: 869.26, y: 701.658), control2: CGPoint(x: 866.646, y: 704.92))
        path.addCurve(to: CGPoint(x: 845.448, y: 723.118), control1: CGPoint(x: 855.755, y: 715.81), control2: CGPoint(x: 852.06, y: 718.524))
        path.addCurve(to: CGPoint(x: 824.448, y: 735.118), control1: CGPoint(x: 837.691, y: 728.507), control2: CGPoint(x: 833.106, y: 731.343))
        path.addCurve(to: CGPoint(x: 802.448, y: 742.118), control1: CGPoint(x: 816.183, y: 738.721), control2: CGPoint(x: 811.238, y: 740.113))
        path.addCurve(to: CGPoint(x: 782.948, y: 745.118), control1: CGPoint(x: 794.936, y: 743.831), control2: CGPoint(x: 790.622, y: 744.434))
        path.addCurve(to: CGPoint(x: 755.448, y: 745.118), control1: CGPoint(x: 772.251, y: 746.071), control2: CGPoint(x: 766.187, y: 745.118))
        path.addLine(to: CGPoint(x: 730.448, y: 745.118))
        path.addLine(to: CGPoint(x: 700.948, y: 745.118))
        path.addCurve(to: CGPoint(x: 677.448, y: 745.118), control1: CGPoint(x: 700.948, y: 745.118), control2: CGPoint(x: 686.618, y: 744.747))
        path.addCurve(to: CGPoint(x: 657.448, y: 746.618), control1: CGPoint(x: 669.622, y: 745.434), control2: CGPoint(x: 665.225, y: 745.686))
        path.addCurve(to: CGPoint(x: 641.448, y: 749.118), control1: CGPoint(x: 651.169, y: 747.37), control2: CGPoint(x: 647.641, y: 747.835))
        path.addCurve(to: CGPoint(x: 619.948, y: 755.118), control1: CGPoint(x: 632.912, y: 750.886), control2: CGPoint(x: 628.135, y: 752.125))
        path.addCurve(to: CGPoint(x: 602.948, y: 762.618), control1: CGPoint(x: 613.133, y: 757.608), control2: CGPoint(x: 609.478, y: 759.454))
        path.addLine(to: CGPoint(x: 602.334, y: 762.915))
        path.addCurve(to: CGPoint(x: 580.948, y: 774.618), control1: CGPoint(x: 593.904, y: 766.999), control2: CGPoint(x: 588.953, y: 769.397))
        path.addCurve(to: CGPoint(x: 565.448, y: 786.118), control1: CGPoint(x: 574.635, y: 778.735), control2: CGPoint(x: 571.172, y: 781.214))
        path.addCurve(to: CGPoint(x: 550.448, y: 801.118), control1: CGPoint(x: 559.157, y: 791.507), control2: CGPoint(x: 555.889, y: 794.871))
        path.addCurve(to: CGPoint(x: 536.948, y: 819.118), control1: CGPoint(x: 544.677, y: 807.744), control2: CGPoint(x: 542.09, y: 811.992))
        path.addCurve(to: CGPoint(x: 525.448, y: 835.618), control1: CGPoint(x: 532.352, y: 825.487), control2: CGPoint(x: 529.486, y: 828.881))
        path.addCurve(to: CGPoint(x: 515.448, y: 855.618), control1: CGPoint(x: 520.958, y: 843.107), control2: CGPoint(x: 518.745, y: 847.531))
        path.addCurve(to: CGPoint(x: 508.448, y: 877.618), control1: CGPoint(x: 512.045, y: 863.966), control2: CGPoint(x: 510.69, y: 868.885))
        path.addCurve(to: CGPoint(x: 504.601, y: 896.241), control1: CGPoint(x: 506.59, y: 884.855), control2: CGPoint(x: 505.845, y: 889.121))
        path.addLine(to: CGPoint(x: 504.448, y: 897.118))
        path.addCurve(to: CGPoint(x: 501.448, y: 918.118), control1: CGPoint(x: 503.021, y: 905.278), control2: CGPoint(x: 502.879, y: 909.958))
        path.addCurve(to: CGPoint(x: 496.448, y: 941.618), control1: CGPoint(x: 499.827, y: 927.359), control2: CGPoint(x: 499.266, y: 932.668))
        path.addCurve(to: CGPoint(x: 487.948, y: 962.118), control1: CGPoint(x: 493.846, y: 949.884), control2: CGPoint(x: 491.882, y: 954.395))
        path.addCurve(to: CGPoint(x: 479.5, y: 976.0), control1: CGPoint(x: 484.969, y: 967.966), control2: CGPoint(x: 483.348, y: 970.682))
        path.addCurve(to: CGPoint(x: 468.448, y: 989.118), control1: CGPoint(x: 475.688, y: 981.268), control2: CGPoint(x: 472.954, y: 984.43))
        path.addCurve(to: CGPoint(x: 453.448, y: 1003.12), control1: CGPoint(x: 462.895, y: 994.894), control2: CGPoint(x: 459.728, y: 998.14))
        path.addCurve(to: CGPoint(x: 433.448, y: 1016.12), control1: CGPoint(x: 446.148, y: 1008.9), control2: CGPoint(x: 441.813, y: 1012.02))
        path.addCurve(to: CGPoint(x: 412.448, y: 1023.62), control1: CGPoint(x: 425.629, y: 1019.95), control2: CGPoint(x: 420.955, y: 1021.76))
        path.addCurve(to: CGPoint(x: 391.948, y: 1025.62), control1: CGPoint(x: 404.59, y: 1025.34), control2: CGPoint(x: 399.983, y: 1025.25))
        path.addCurve(to: CGPoint(x: 373.448, y: 1025.62), control1: CGPoint(x: 384.731, y: 1025.95), control2: CGPoint(x: 380.65, y: 1026.19))
        path.addCurve(to: CGPoint(x: 352.948, y: 1022.12), control1: CGPoint(x: 365.352, y: 1024.97), control2: CGPoint(x: 360.78, y: 1024.27))
        path.addCurve(to: CGPoint(x: 332.448, y: 1014.12), control1: CGPoint(x: 344.66, y: 1019.84), control2: CGPoint(x: 340.037, y: 1018.15))
        path.addCurve(to: CGPoint(x: 317.948, y: 1004.62), control1: CGPoint(x: 326.47, y: 1010.94), control2: CGPoint(x: 323.373, y: 1008.67))
        path.addCurve(to: CGPoint(x: 305.448, y: 994.118), control1: CGPoint(x: 312.839, y: 1000.8), control2: CGPoint(x: 310.001, y: 998.58))
        path.addCurve(to: CGPoint(x: 293.448, y: 980.118), control1: CGPoint(x: 300.306, y: 989.077), control2: CGPoint(x: 297.646, y: 985.969))
        path.addCurve(to: CGPoint(x: 284.448, y: 965.118), control1: CGPoint(x: 289.466, y: 974.567), control2: CGPoint(x: 287.567, y: 971.195))
        path.addCurve(to: CGPoint(x: 278.448, y: 951.618), control1: CGPoint(x: 281.814, y: 959.985), control2: CGPoint(x: 280.379, y: 957.054))
        path.addCurve(to: CGPoint(x: 274.448, y: 936.618), control1: CGPoint(x: 276.418, y: 945.905), control2: CGPoint(x: 275.599, y: 942.57))
        path.addCurve(to: CGPoint(x: 272.456, y: 918.269), control1: CGPoint(x: 273.078, y: 929.533), control2: CGPoint(x: 272.851, y: 925.431))
        path.addLine(to: CGPoint(x: 272.448, y: 918.118))
        path.addCurve(to: CGPoint(x: 272.448, y: 898.618), control1: CGPoint(x: 272.029, y: 910.514), control2: CGPoint(x: 271.695, y: 906.196))
        path.addCurve(to: CGPoint(x: 275.948, y: 880.618), control1: CGPoint(x: 273.156, y: 891.492), control2: CGPoint(x: 274.023, y: 887.515))
        path.addCurve(to: CGPoint(x: 281.948, y: 864.118), control1: CGPoint(x: 277.791, y: 874.013), control2: CGPoint(x: 279.121, y: 870.364))
        path.addCurve(to: CGPoint(x: 291.448, y: 847.118), control1: CGPoint(x: 285.084, y: 857.189), control2: CGPoint(x: 287.318, y: 853.503))
        path.addCurve(to: CGPoint(x: 306.448, y: 827.618), control1: CGPoint(x: 296.666, y: 839.05), control2: CGPoint(x: 299.701, y: 834.457))
        path.addCurve(to: CGPoint(x: 321.448, y: 815.118), control1: CGPoint(x: 311.803, y: 822.189), control2: CGPoint(x: 315.181, y: 819.461))
        path.addCurve(to: CGPoint(x: 343.948, y: 803.118), control1: CGPoint(x: 329.633, y: 809.446), control2: CGPoint(x: 334.737, y: 806.903))
        path.addCurve(to: CGPoint(x: 362.448, y: 797.118), control1: CGPoint(x: 350.973, y: 800.231), control2: CGPoint(x: 355.076, y: 798.945))
        path.addCurve(to: CGPoint(x: 391.948, y: 793.118), control1: CGPoint(x: 373.732, y: 794.32), control2: CGPoint(x: 380.373, y: 794.2))
        path.closeSubpath()
        path.move(to: CGPoint(x: 352.035, y: 527.5))
        path.addCurve(to: CGPoint(x: 376.035, y: 522.0), control1: CGPoint(x: 361.182, y: 524.534), control2: CGPoint(x: 366.6, y: 523.854))
        path.addCurve(to: CGPoint(x: 387.035, y: 520.0), control1: CGPoint(x: 380.32, y: 521.158), control2: CGPoint(x: 382.819, y: 521.134))
        path.addCurve(to: CGPoint(x: 401.035, y: 514.5), control1: CGPoint(x: 392.708, y: 518.474), control2: CGPoint(x: 395.721, y: 517.003))
        path.addCurve(to: CGPoint(x: 416.035, y: 506.0), control1: CGPoint(x: 407.127, y: 511.631), control2: CGPoint(x: 410.416, y: 509.709))
        path.addCurve(to: CGPoint(x: 432.035, y: 493.5), control1: CGPoint(x: 422.653, y: 501.632), control2: CGPoint(x: 426.22, y: 498.89))
        path.addCurve(to: CGPoint(x: 444.535, y: 480.0), control1: CGPoint(x: 437.305, y: 488.616), control2: CGPoint(x: 440.083, y: 485.639))
        path.addCurve(to: CGPoint(x: 454.035, y: 466.0), control1: CGPoint(x: 448.63, y: 474.815), control2: CGPoint(x: 450.622, y: 471.657))
        path.addCurve(to: CGPoint(x: 464.035, y: 446.5), control1: CGPoint(x: 458.457, y: 458.672), control2: CGPoint(x: 460.703, y: 454.383))
        path.addCurve(to: CGPoint(x: 470.035, y: 429.5), control1: CGPoint(x: 466.777, y: 440.016), control2: CGPoint(x: 467.875, y: 436.201))
        path.addCurve(to: CGPoint(x: 475.535, y: 410.5), control1: CGPoint(x: 472.406, y: 422.148), control2: CGPoint(x: 473.8, y: 418.027))
        path.addCurve(to: CGPoint(x: 478.535, y: 394.0), control1: CGPoint(x: 477.007, y: 404.118), control2: CGPoint(x: 477.658, y: 400.49))
        path.addCurve(to: CGPoint(x: 480.035, y: 377.0), control1: CGPoint(x: 479.428, y: 387.395), control2: CGPoint(x: 479.699, y: 383.656))
        path.addCurve(to: CGPoint(x: 480.035, y: 354.0), control1: CGPoint(x: 480.489, y: 368.029), control2: CGPoint(x: 479.486, y: 362.965))
        path.addCurve(to: CGPoint(x: 480.986, y: 343.458), control1: CGPoint(x: 480.29, y: 349.85), control2: CGPoint(x: 480.55, y: 347.465))
        path.addLine(to: CGPoint(x: 481.035, y: 343.0))
        path.addCurve(to: CGPoint(x: 483.035, y: 328.0), control1: CGPoint(x: 481.674, y: 337.125), control2: CGPoint(x: 481.919, y: 333.803))
        path.addCurve(to: CGPoint(x: 486.535, y: 314.0), control1: CGPoint(x: 484.1, y: 322.466), control2: CGPoint(x: 484.85, y: 319.378))
        path.addCurve(to: CGPoint(x: 493.535, y: 296.5), control1: CGPoint(x: 488.737, y: 306.976), control2: CGPoint(x: 490.327, y: 303.125))
        path.addCurve(to: CGPoint(x: 502.035, y: 281.5), control1: CGPoint(x: 496.47, y: 290.44), control2: CGPoint(x: 498.313, y: 287.11))
        path.addCurve(to: CGPoint(x: 511.535, y: 269.0), control1: CGPoint(x: 505.425, y: 276.391), control2: CGPoint(x: 507.522, y: 273.635))
        path.addCurve(to: CGPoint(x: 523.499, y: 256.992), control1: CGPoint(x: 515.893, y: 263.967), control2: CGPoint(x: 518.739, y: 261.357))
        path.addLine(to: CGPoint(x: 524.035, y: 256.5))
        path.addCurve(to: CGPoint(x: 534.535, y: 247.5), control1: CGPoint(x: 528.015, y: 252.849), control2: CGPoint(x: 530.186, y: 250.701))
        path.addCurve(to: CGPoint(x: 546.535, y: 240.0), control1: CGPoint(x: 538.986, y: 244.224), control2: CGPoint(x: 541.732, y: 242.732))
        path.addCurve(to: CGPoint(x: 557.035, y: 234.5), control1: CGPoint(x: 550.559, y: 237.711), control2: CGPoint(x: 552.837, y: 236.45))
        path.addCurve(to: CGPoint(x: 569.535, y: 229.5), control1: CGPoint(x: 561.804, y: 232.286), control2: CGPoint(x: 564.534, y: 231.123))
        path.addCurve(to: CGPoint(x: 587.035, y: 225.5), control1: CGPoint(x: 576.204, y: 227.336), control2: CGPoint(x: 580.113, y: 226.607))
        path.addCurve(to: CGPoint(x: 601.035, y: 224.0), control1: CGPoint(x: 592.465, y: 224.632), control2: CGPoint(x: 601.035, y: 224.0))
        path.addLine(to: CGPoint(x: 611.035, y: 224.0))
        path.addLine(to: CGPoint(x: 631.535, y: 224.0))
        path.addLine(to: CGPoint(x: 657.035, y: 224.0))
        path.addLine(to: CGPoint(x: 685.035, y: 224.0))
        path.addLine(to: CGPoint(x: 722.035, y: 224.0))
        path.addLine(to: CGPoint(x: 753.535, y: 224.0))
        path.addLine(to: CGPoint(x: 803.535, y: 224.0))
        path.addLine(to: CGPoint(x: 855.535, y: 224.0))
        path.addLine(to: CGPoint(x: 898.535, y: 224.0))
        path.addCurve(to: CGPoint(x: 908.535, y: 225.0), control1: CGPoint(x: 898.535, y: 224.0), control2: CGPoint(x: 904.655, y: 224.41))
        path.addCurve(to: CGPoint(x: 923.535, y: 228.5), control1: CGPoint(x: 914.482, y: 225.904), control2: CGPoint(x: 917.754, y: 226.838))
        path.addCurve(to: CGPoint(x: 940.535, y: 234.5), control1: CGPoint(x: 930.302, y: 230.445), control2: CGPoint(x: 934.076, y: 231.7))
        path.addCurve(to: CGPoint(x: 956.035, y: 242.5), control1: CGPoint(x: 946.785, y: 237.209), control2: CGPoint(x: 950.334, y: 238.773))
        path.addCurve(to: CGPoint(x: 971.535, y: 255.5), control1: CGPoint(x: 962.648, y: 246.823), control2: CGPoint(x: 965.908, y: 249.955))
        path.addCurve(to: CGPoint(x: 984.535, y: 270.5), control1: CGPoint(x: 977.057, y: 260.941), control2: CGPoint(x: 980.109, y: 264.137))
        path.addCurve(to: CGPoint(x: 993.035, y: 285.5), control1: CGPoint(x: 988.38, y: 276.027), control2: CGPoint(x: 990.076, y: 279.452))
        path.addCurve(to: CGPoint(x: 1000.04, y: 302.5), control1: CGPoint(x: 996.191, y: 291.949), control2: CGPoint(x: 997.894, y: 295.647))
        path.addCurve(to: CGPoint(x: 1003.54, y: 317.5), control1: CGPoint(x: 1001.83, y: 308.242), control2: CGPoint(x: 1002.7, y: 311.495))
        path.addCurve(to: CGPoint(x: 1004.04, y: 335.0), control1: CGPoint(x: 1004.48, y: 324.269), control2: CGPoint(x: 1004.36, y: 328.174))
        path.addCurve(to: CGPoint(x: 1002.04, y: 354.5), control1: CGPoint(x: 1003.67, y: 342.646), control2: CGPoint(x: 1003.44, y: 346.975))
        path.addCurve(to: CGPoint(x: 998.035, y: 370.0), control1: CGPoint(x: 1000.89, y: 360.645), control2: CGPoint(x: 1000.02, y: 364.072))
        path.addCurve(to: CGPoint(x: 991.535, y: 385.5), control1: CGPoint(x: 995.952, y: 376.224), control2: CGPoint(x: 994.531, y: 379.659))
        path.addCurve(to: CGPoint(x: 984.035, y: 398.0), control1: CGPoint(x: 988.938, y: 390.566), control2: CGPoint(x: 987.32, y: 393.35))
        path.addCurve(to: CGPoint(x: 973.035, y: 411.0), control1: CGPoint(x: 980.198, y: 403.432), control2: CGPoint(x: 977.63, y: 406.192))
        path.addCurve(to: CGPoint(x: 960.035, y: 423.0), control1: CGPoint(x: 968.262, y: 415.995), control2: CGPoint(x: 965.469, y: 418.733))
        path.addCurve(to: CGPoint(x: 943.035, y: 434.0), control1: CGPoint(x: 953.816, y: 427.883), control2: CGPoint(x: 949.993, y: 430.242))
        path.addCurve(to: CGPoint(x: 925.035, y: 442.0), control1: CGPoint(x: 936.267, y: 437.655), control2: CGPoint(x: 932.319, y: 439.526))
        path.addCurve(to: CGPoint(x: 903.035, y: 447.0), control1: CGPoint(x: 916.693, y: 444.834), control2: CGPoint(x: 911.77, y: 445.848))
        path.addCurve(to: CGPoint(x: 890.535, y: 448.0), control1: CGPoint(x: 898.18, y: 447.64), control2: CGPoint(x: 890.535, y: 448.0))
        path.addLine(to: CGPoint(x: 871.035, y: 448.0))
        path.addLine(to: CGPoint(x: 846.035, y: 448.0))
        path.addLine(to: CGPoint(x: 816.535, y: 448.0))
        path.addLine(to: CGPoint(x: 788.535, y: 448.0))
        path.addLine(to: CGPoint(x: 761.535, y: 448.0))
        path.addLine(to: CGPoint(x: 733.535, y: 448.0))
        path.addLine(to: CGPoint(x: 702.035, y: 448.0))
        path.addLine(to: CGPoint(x: 667.535, y: 448.0))
        path.addLine(to: CGPoint(x: 635.535, y: 448.0))
        path.addCurve(to: CGPoint(x: 617.035, y: 450.5), control1: CGPoint(x: 635.535, y: 448.0), control2: CGPoint(x: 624.158, y: 448.944))
        path.addCurve(to: CGPoint(x: 599.035, y: 456.0), control1: CGPoint(x: 609.854, y: 452.069), control2: CGPoint(x: 605.855, y: 453.258))
        path.addCurve(to: CGPoint(x: 583.047, y: 463.993), control1: CGPoint(x: 592.558, y: 458.604), control2: CGPoint(x: 589.118, y: 460.553))
        path.addLine(to: CGPoint(x: 583.035, y: 464.0))
        path.addCurve(to: CGPoint(x: 566.035, y: 475.0), control1: CGPoint(x: 576.156, y: 467.898), control2: CGPoint(x: 572.406, y: 470.315))
        path.addCurve(to: CGPoint(x: 552.035, y: 486.5), control1: CGPoint(x: 560.336, y: 479.192), control2: CGPoint(x: 557.177, y: 481.639))
        path.addCurve(to: CGPoint(x: 540.035, y: 499.5), control1: CGPoint(x: 547.015, y: 491.246), control2: CGPoint(x: 544.436, y: 494.174))
        path.addCurve(to: CGPoint(x: 529.035, y: 514.5), control1: CGPoint(x: 535.409, y: 505.1), control2: CGPoint(x: 532.919, y: 508.361))
        path.addCurve(to: CGPoint(x: 521.535, y: 528.0), control1: CGPoint(x: 525.811, y: 519.597), control2: CGPoint(x: 524.323, y: 522.652))
        path.addCurve(to: CGPoint(x: 514.035, y: 543.5), control1: CGPoint(x: 518.428, y: 533.963), control2: CGPoint(x: 516.743, y: 537.345))
        path.addCurve(to: CGPoint(x: 508.035, y: 558.5), control1: CGPoint(x: 511.495, y: 549.275), control2: CGPoint(x: 509.966, y: 552.493))
        path.addCurve(to: CGPoint(x: 504.035, y: 575.0), control1: CGPoint(x: 506.007, y: 564.812), control2: CGPoint(x: 505.321, y: 568.496))
        path.addCurve(to: CGPoint(x: 501.035, y: 596.0), control1: CGPoint(x: 502.429, y: 583.127), control2: CGPoint(x: 501.953, y: 587.767))
        path.addCurve(to: CGPoint(x: 499.564, y: 613.086), control1: CGPoint(x: 500.291, y: 602.678), control2: CGPoint(x: 500.024, y: 606.5))
        path.addLine(to: CGPoint(x: 499.535, y: 613.5))
        path.addCurve(to: CGPoint(x: 498.535, y: 632.5), control1: CGPoint(x: 499.018, y: 620.912), control2: CGPoint(x: 499.119, y: 625.093))
        path.addCurve(to: CGPoint(x: 496.535, y: 651.5), control1: CGPoint(x: 497.949, y: 639.938), control2: CGPoint(x: 497.836, y: 644.153))
        path.addCurve(to: CGPoint(x: 492.535, y: 668.0), control1: CGPoint(x: 495.379, y: 658.029), control2: CGPoint(x: 494.69, y: 661.729))
        path.addCurve(to: CGPoint(x: 485.535, y: 683.5), control1: CGPoint(x: 490.377, y: 674.281), control2: CGPoint(x: 488.678, y: 677.649))
        path.addCurve(to: CGPoint(x: 476.035, y: 698.5), control1: CGPoint(x: 482.255, y: 689.609), control2: CGPoint(x: 480.099, y: 692.881))
        path.addCurve(to: CGPoint(x: 464.035, y: 713.0), control1: CGPoint(x: 471.728, y: 704.456), control2: CGPoint(x: 469.113, y: 707.686))
        path.addCurve(to: CGPoint(x: 451.162, y: 723.881), control1: CGPoint(x: 459.499, y: 717.747), control2: CGPoint(x: 456.406, y: 720.023))
        path.addLine(to: CGPoint(x: 451.0, y: 724.0))
        path.addCurve(to: CGPoint(x: 439.035, y: 732.0), control1: CGPoint(x: 446.415, y: 727.374), control2: CGPoint(x: 443.997, y: 729.208))
        path.addCurve(to: CGPoint(x: 421.0, y: 740.5), control1: CGPoint(x: 432.338, y: 735.769), control2: CGPoint(x: 428.225, y: 737.882))
        path.addCurve(to: CGPoint(x: 405.035, y: 745.0), control1: CGPoint(x: 414.766, y: 742.759), control2: CGPoint(x: 411.568, y: 743.868))
        path.addCurve(to: CGPoint(x: 393.035, y: 746.5), control1: CGPoint(x: 400.382, y: 745.806), control2: CGPoint(x: 397.744, y: 746.134))
        path.addCurve(to: CGPoint(x: 373.035, y: 746.5), control1: CGPoint(x: 385.248, y: 747.105), control2: CGPoint(x: 380.829, y: 747.014))
        path.addCurve(to: CGPoint(x: 359.535, y: 745.0), control1: CGPoint(x: 367.742, y: 746.151), control2: CGPoint(x: 364.736, y: 746.042))
        path.addCurve(to: CGPoint(x: 346.269, y: 741.084), control1: CGPoint(x: 354.226, y: 743.936), control2: CGPoint(x: 351.301, y: 742.888))
        path.addLine(to: CGPoint(x: 346.035, y: 741.0))
        path.addCurve(to: CGPoint(x: 332.035, y: 735.0), control1: CGPoint(x: 340.436, y: 738.993), control2: CGPoint(x: 337.364, y: 737.643))
        path.addCurve(to: CGPoint(x: 318.035, y: 727.0), control1: CGPoint(x: 326.394, y: 732.202), control2: CGPoint(x: 323.238, y: 730.548))
        path.addCurve(to: CGPoint(x: 304.035, y: 715.5), control1: CGPoint(x: 312.19, y: 723.014), control2: CGPoint(x: 308.973, y: 720.567))
        path.addCurve(to: CGPoint(x: 295.035, y: 704.5), control1: CGPoint(x: 300.162, y: 711.525), control2: CGPoint(x: 298.342, y: 708.958))
        path.addCurve(to: CGPoint(x: 285.535, y: 690.0), control1: CGPoint(x: 291.003, y: 699.062), control2: CGPoint(x: 288.87, y: 695.891))
        path.addCurve(to: CGPoint(x: 278.035, y: 674.5), control1: CGPoint(x: 282.223, y: 684.148), control2: CGPoint(x: 280.483, y: 680.763))
        path.addCurve(to: CGPoint(x: 273.035, y: 658.0), control1: CGPoint(x: 275.585, y: 668.229), control2: CGPoint(x: 274.237, y: 664.625))
        path.addCurve(to: CGPoint(x: 272.035, y: 639.0), control1: CGPoint(x: 271.71, y: 650.689), control2: CGPoint(x: 272.035, y: 646.43))
        path.addLine(to: CGPoint(x: 272.035, y: 638.915))
        path.addCurve(to: CGPoint(x: 273.035, y: 620.0), control1: CGPoint(x: 272.035, y: 631.538), control2: CGPoint(x: 272.035, y: 627.334))
        path.addCurve(to: CGPoint(x: 276.535, y: 603.5), control1: CGPoint(x: 273.925, y: 613.473), control2: CGPoint(x: 274.576, y: 609.789))
        path.addCurve(to: CGPoint(x: 280.535, y: 593.0), control1: CGPoint(x: 277.841, y: 599.311), control2: CGPoint(x: 278.834, y: 597.045))
        path.addLine(to: CGPoint(x: 280.678, y: 592.661))
        path.addCurve(to: CGPoint(x: 288.035, y: 577.5), control1: CGPoint(x: 283.197, y: 586.672), control2: CGPoint(x: 284.667, y: 583.177))
        path.addCurve(to: CGPoint(x: 298.035, y: 563.5), control1: CGPoint(x: 291.464, y: 571.722), control2: CGPoint(x: 293.741, y: 568.667))
        path.addCurve(to: CGPoint(x: 309.535, y: 551.5), control1: CGPoint(x: 302.184, y: 558.508), control2: CGPoint(x: 304.68, y: 555.807))
        path.addCurve(to: CGPoint(x: 320.535, y: 543.0), control1: CGPoint(x: 313.597, y: 547.897), control2: CGPoint(x: 316.077, y: 546.098))
        path.addCurve(to: CGPoint(x: 333.535, y: 535.0), control1: CGPoint(x: 325.431, y: 539.599), control2: CGPoint(x: 328.247, y: 537.75))
        path.addCurve(to: CGPoint(x: 352.035, y: 527.5), control1: CGPoint(x: 340.452, y: 531.403), control2: CGPoint(x: 344.62, y: 529.905))
        path.closeSubpath()
        let scale = min(rect.width / 732.7850000000001, rect.height / 802.19)
        let transform = CGAffineTransform(a: scale, b: 0, c: 0, d: scale,
                                         tx: rect.midX - 638.0875000000001 * scale,
                                         ty: rect.midY - 625.095 * scale)
        return path.applying(transform)
    }
}

private extension Color {
    init(widgetHex value: UInt32) {
        self.init(red: Double((value >> 16) & 255) / 255,
                  green: Double((value >> 8) & 255) / 255,
                  blue: Double(value & 255) / 255)
    }
}

struct FlowWidgetPalette {
    let dark: Bool
    var surface: Color { Color(widgetHex: dark ? 0x17191d : 0xffffff) }
    var ink: Color { Color(widgetHex: dark ? 0xf4f5f7 : 0x181b20) }
    var secondary: Color { Color(widgetHex: dark ? 0xa1a8b2 : 0x626a75) }
    var border: Color { Color(widgetHex: dark ? 0x30343c : 0xe8ebef) }
    var inset: Color { Color(widgetHex: dark ? 0x22262d : 0xf4f6f8) }
    var accentText: Color { Color(widgetHex: dark ? 0xffb576 : 0xa44713) }
    static let coral = Color(widgetHex: 0xfb562d)
    static let gradient = LinearGradient(colors: [Color(widgetHex: 0xfb923c), Color(widgetHex: 0xef4444)],
                                         startPoint: .topLeading, endPoint: .bottomTrailing)
}

struct FlowWidgetBackground: View {
    let kind: String
    @Environment(\.colorScheme) private var colorScheme
    var body: some View { FlowWidgetPalette(dark: colorScheme == .dark).surface }
}

struct FlowWidgetHeader: View {
    let title: String
    @Environment(\.colorScheme) private var colorScheme
    var body: some View {
        HStack(spacing: 8) {
            Text(title).font(.subheadline.weight(.semibold)).lineLimit(1).minimumScaleFactor(0.8)
                .foregroundStyle(FlowWidgetPalette(dark: colorScheme == .dark).ink)
            Spacer(minLength: 0)
            FlowLikeMark().fill(FlowWidgetPalette.gradient).frame(width: 15, height: 17)
                .widgetAccentable().accessibilityHidden(true)
        }
    }
}

struct FlowAppIcon: View {
    let scope: String
    let appId: String
    let title: String
    let size: CGFloat
    @Environment(\.colorScheme) private var colorScheme

    private var image: Image? {
        guard let data = NativeStore.shared.appIconData(scope: scope, appId: appId) else { return nil }
        #if os(iOS)
        guard let image = UIImage(data: data) else { return nil }
        return Image(uiImage: image)
        #else
        guard let image = NSImage(data: data) else { return nil }
        return Image(nsImage: image)
        #endif
    }

    var body: some View {
        let palette = FlowWidgetPalette(dark: colorScheme == .dark)
        Group {
            if let image {
                image.resizable().scaledToFit()
            } else {
                let initials = title.split(whereSeparator: { $0.isWhitespace })
                    .prefix(2).compactMap(\.first).map(String.init).joined().uppercased()
                if initials.isEmpty {
                    Image(systemName: "app").font(.system(size: size * 0.44, weight: .medium))
                } else {
                    Text(initials).font(.system(size: size * 0.31, weight: .semibold, design: .rounded))
                        .lineLimit(1).minimumScaleFactor(0.7)
                }
            }
        }
        .frame(width: size, height: size)
        .foregroundStyle(palette.ink)
        .background(palette.inset, in: RoundedRectangle(cornerRadius: size * 0.25))
        .clipShape(RoundedRectangle(cornerRadius: size * 0.25))
        .accessibilityHidden(true)
    }
}

struct FlowNotificationIcon: View {
    let item: NativeItem
    let scope: String
    let size: CGFloat
    @Environment(\.colorScheme) private var colorScheme

    private func image(_ data: Data) -> Image? {
        #if os(iOS)
        guard let image = UIImage(data: data) else { return nil }
        return Image(uiImage: image)
        #else
        guard let image = NSImage(data: data) else { return nil }
        return Image(nsImage: image)
        #endif
    }

    var body: some View {
        let palette = FlowWidgetPalette(dark: colorScheme == .dark)
        Group {
            switch NativeStore.shared.notificationIcon(for: item, scope: scope) {
            case let .image(data, template):
                if let image = image(data) {
                    image.renderingMode(template ? .template : .original).resizable().scaledToFit()
                        .padding(template ? size * 0.18 : 0)
                        .foregroundStyle(palette.accentText)
                } else { brand }
            case let .text(value):
                Text(value).font(.system(size: size * 0.65)).lineLimit(1).minimumScaleFactor(0.6)
            case .flowLike:
                brand
            }
        }
        .frame(width: size, height: size)
        .background(palette.inset, in: RoundedRectangle(cornerRadius: size * 0.25))
        .clipShape(RoundedRectangle(cornerRadius: size * 0.25))
        .accessibilityHidden(true)
    }

    private var brand: some View {
        FlowLikeMark().fill(FlowWidgetPalette.gradient)
            .frame(width: size * 0.5, height: size * 0.57).widgetAccentable()
    }
}

struct WorkspaceWidgetView: View {
    let entry: WorkspaceEntry
    let sectionKind: String
    let title: String
    let symbol: String
    @Environment(\.widgetFamily) private var family
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Environment(\.colorScheme) private var colorScheme

    private var palette: FlowWidgetPalette { FlowWidgetPalette(dark: colorScheme == .dark) }
    private var section: NativeSection? { entry.snapshot?.sections.first { $0.kind == sectionKind } }
    private var items: [NativeItem] { section?.state == "ready" ? section?.items ?? [] : [] }
    private var small: Bool { family == .systemSmall }
    private var large: Bool { family == .systemLarge }
    private var accessible: Bool { dynamicTypeSize.isAccessibilitySize }
    private var rowLimit: Int { accessible ? (large ? 3 : 1) : (large ? 5 : (small ? 1 : 2)) }
    private var displayTitle: String {
        switch sectionKind {
        case "workspace": return "Workspace"
        case "inbox": return "Inbox"
        case "event_favorites": return "Favorites"
        case "recent_apps": return small && accessible ? "Apps" : title
        case "recent_runs": return small && accessible ? "Runs" : title
        case "attention": return small ? "Attention" : "Needs attention"
        default: return title
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: accessible ? 6 : 10) {
            FlowWidgetHeader(title: displayTitle).accessibilityLabel(title)
            if sectionKind == "flowpilot", section?.state == "ready" {
                flowPilotContent
            } else if let snapshot = entry.snapshot, !items.isEmpty {
                switch sectionKind {
                case "usage": usageContent(snapshot)
                case "workspace": workspaceContent(snapshot)
                case "recent_apps" where !accessible: appGrid(snapshot)
                case "event_favorites" where !small && !accessible: favoriteGrid(snapshot)
                default: itemList(snapshot)
                }
            } else { emptyContent }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .foregroundStyle(palette.ink)
        .containerBackground(for: .widget) { FlowWidgetBackground(kind: sectionKind) }
        .widgetURL(NativeWidgetLaunchURL.section(sectionKind, scope: entry.snapshot?.scope))
    }

    private var voiceIntent: OpenFlowPilotIntent { OpenFlowPilotIntent(voice: true) }

    private var flowPilotContent: some View {
        VStack(alignment: .leading, spacing: 10) {
            Spacer(minLength: 0)
            HStack(spacing: 8) {
                Button(intent: OpenFlowPilotIntent()) {
                    VStack(alignment: .leading, spacing: 14) {
                        HStack(spacing: 8) {
                            Image(systemName: "text.bubble").font(.body).foregroundStyle(palette.accentText)
                            if small && !accessible { Text("Ask").font(.subheadline.weight(.medium)) }
                            if !small { Spacer(minLength: 0); Image(systemName: "arrow.up.right").font(.caption).foregroundStyle(palette.secondary) }
                        }
                        if !small { Text(accessible ? "Ask" : "Ask a question").font(.subheadline.weight(.semibold)).lineLimit(1) }
                    }
                    .foregroundStyle(palette.ink).padding(.horizontal, 12)
                    .frame(maxWidth: .infinity, minHeight: small ? 44 : 96, alignment: .leading)
                    .background(palette.inset, in: RoundedRectangle(cornerRadius: 12))
                }.buttonStyle(.plain).accessibilityLabel("Start a new FlowPilot conversation")
                Button(intent: voiceIntent) {
                    VStack(alignment: .leading, spacing: 14) {
                        Image(systemName: "mic.fill").font(.body.weight(.medium)).foregroundStyle(palette.accentText)
                        if !small { Text("Voice").font(.subheadline.weight(.semibold)).lineLimit(1) }
                    }
                        .foregroundStyle(palette.ink).padding(.horizontal, small ? 0 : 12)
                        .frame(width: small ? 44 : (accessible ? 108 : 100), height: small ? 44 : 96, alignment: small ? .center : .leading)
                        .background(palette.inset, in: RoundedRectangle(cornerRadius: 12))
                }.buttonStyle(.plain).accessibilityLabel("Open FlowPilot voice input")
            }
            if small && !accessible {
                Text("Type or talk").font(.caption).foregroundStyle(palette.secondary)
            }
        }
    }

    @ViewBuilder private func usageContent(_ snapshot: NativeSnapshot) -> some View {
        if let primary = items.first(where: { $0.id == "executions" }) ?? items.first {
            if small || accessible {
                metric(primary, snapshot: snapshot, size: 36, label: "Executions")
                if !accessible { Text("Account · all recorded").font(.caption2).foregroundStyle(palette.secondary) }
                Spacer(minLength: 0)
            } else {
                HStack(alignment: .center, spacing: 18) {
                    VStack(alignment: .leading, spacing: 2) {
                        metric(primary, snapshot: snapshot, size: 40, label: "Executions")
                        Text("Account · all recorded").font(.caption2).foregroundStyle(palette.secondary)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                    Rectangle().fill(palette.border).frame(width: 1, height: 72)
                    VStack(spacing: 13) {
                        ForEach(Array(items.filter { $0.id != primary.id }.prefix(2))) { item in
                            Button(intent: OpenNativeItemIntent(itemId: item.id, section: sectionKind, scope: snapshot.scope)) {
                                HStack(alignment: .firstTextBaseline) {
                                    Text(item.id == "cost" ? "AI cost · USD" : "Model calls")
                                        .font(.caption2).foregroundStyle(palette.secondary).lineLimit(1)
                                    Spacer(minLength: 8)
                                    Text(metricValue(item)).font(.title3.weight(.semibold)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.75)
                                }.contentShape(Rectangle())
                            }.buttonStyle(.plain).privacySensitive()
                        }
                    }.frame(maxWidth: .infinity)
                }.frame(maxHeight: .infinity)
            }
        }
    }

    private func workspaceContent(_ snapshot: NativeSnapshot) -> some View {
        VStack(alignment: .leading, spacing: large ? 18 : 8) {
            HStack(alignment: .top, spacing: 20) {
                ForEach(Array(items.prefix(small || accessible ? 1 : 2))) { item in
                    metric(item, snapshot: snapshot, size: small ? 38 : 36,
                           label: item.id == "activity" ? "Account runs · 7 days" : "Apps in profile")
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }.privacySensitive()
            if small && !accessible, let activity = items.first(where: { $0.id == "activity" }) {
                Text("\(metricValue(activity)) account runs · 7d").font(.caption).foregroundStyle(palette.secondary).lineLimit(1)
            }
            if large {
                Rectangle().fill(palette.border).frame(height: 1)
                let recent = snapshot.sections.first { $0.kind == "recent_apps" && $0.state == "ready" }?.items ?? []
                Text("RECENTLY OPENED").font(.caption2.weight(.semibold)).tracking(0.8).foregroundStyle(palette.secondary)
                if recent.isEmpty {
                    Text("Open an app to see it here.").font(.caption).foregroundStyle(palette.secondary)
                } else {
                    ForEach(Array(recent.prefix(accessible ? 1 : 3))) { item in
                        Button(intent: OpenNativeItemIntent(itemId: item.id, section: "recent_apps", scope: snapshot.scope)) {
                            HStack(spacing: 10) {
                                FlowAppIcon(scope: snapshot.scope, appId: item.action.appId ?? item.id, title: item.title, size: 30)
                                Text(item.title).font(.subheadline.weight(.medium)).lineLimit(1)
                                Spacer(minLength: 0)
                                Image(systemName: "arrow.up.right").font(.caption).foregroundStyle(palette.secondary)
                            }.contentShape(Rectangle())
                        }.buttonStyle(.plain).privacySensitive()
                    }
                }
            }
            Spacer(minLength: 0)
        }.privacySensitive()
    }

    private func metric(_ item: NativeItem, snapshot: NativeSnapshot, size: CGFloat, label: String) -> some View {
        Button(intent: OpenNativeItemIntent(itemId: item.id, section: sectionKind, scope: snapshot.scope)) {
            VStack(alignment: .leading, spacing: 0) {
                Text(metricValue(item)).font(.system(size: accessible ? size + 4 : size, weight: .semibold, design: .rounded))
                    .monospacedDigit().tracking(-1).lineLimit(1).minimumScaleFactor(0.65)
                Text(label).font(.caption).foregroundStyle(palette.secondary).lineLimit(2)
            }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
        }.buttonStyle(.plain).privacySensitive()
    }

    private func metricValue(_ item: NativeItem) -> String {
        guard let value = item.value else { return "N/A" }
        if let integer = Int(value) { return integer.formatted() }
        return value
    }

    private func appGrid(_ snapshot: NativeSnapshot) -> some View {
        let columns = small ? 1 : (large ? 2 : 3)
        let count = small ? 1 : (large ? 6 : 3)
        return LazyVGrid(columns: Array(repeating: GridItem(.flexible(), alignment: .topLeading), count: columns), alignment: .leading, spacing: large ? 18 : 8) {
            ForEach(Array(items.prefix(count))) { item in
                Button(intent: OpenNativeItemIntent(itemId: item.id, section: sectionKind, scope: snapshot.scope)) {
                    VStack(alignment: .leading, spacing: 5) {
                        FlowAppIcon(scope: snapshot.scope, appId: item.action.appId ?? item.id, title: item.title, size: small ? 34 : 32)
                        Text(item.title).font(small ? .subheadline.weight(.semibold) : .caption.weight(.semibold))
                            .lineLimit(2).multilineTextAlignment(.leading)
                            .frame(minHeight: small ? nil : 30, alignment: .topLeading)
                        if !small { itemTime(item).font(.caption2).foregroundStyle(palette.secondary) }
                    }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                }.buttonStyle(.plain).privacySensitive()
            }
        }
    }

    private func favoriteGrid(_ snapshot: NativeSnapshot) -> some View {
        LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 10) {
            ForEach(Array(items.prefix(large ? 4 : 2))) { item in
                Button(intent: OpenNativeItemIntent(itemId: item.id, section: sectionKind, scope: snapshot.scope)) {
                    VStack(alignment: .leading, spacing: large ? 10 : 6) {
                        HStack {
                            Image(systemName: item.action.kind == "run_event" ? "play.fill" : "arrow.up.right")
                                .font(.caption).foregroundStyle(palette.accentText)
                            Text(item.action.kind == "run_event" ? "Run" : "Open").font(.caption2.weight(.medium)).foregroundStyle(palette.secondary)
                            Spacer(minLength: 0)
                        }
                        Text(item.title).font(.subheadline.weight(.semibold)).lineLimit(2).multilineTextAlignment(.leading)
                        if large, let app = item.subtitle { Text(app).font(.caption).foregroundStyle(palette.secondary).lineLimit(1) }
                    }.padding(10).frame(maxWidth: .infinity, minHeight: large ? 113 : 84, alignment: .topLeading)
                        .background(palette.inset, in: RoundedRectangle(cornerRadius: 12)).contentShape(Rectangle())
                }.buttonStyle(.plain).privacySensitive()
            }
        }
    }

    private func itemList(_ snapshot: NativeSnapshot) -> some View {
        ViewThatFits(in: .vertical) {
            itemRows(snapshot, limit: rowLimit)
            if rowLimit > 4 { itemRows(snapshot, limit: 4) }
            if rowLimit > 3 { itemRows(snapshot, limit: 3) }
            if rowLimit > 2 { itemRows(snapshot, limit: 2) }
            if rowLimit > 1 { itemRows(snapshot, limit: 1) }
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func itemRows(_ snapshot: NativeSnapshot, limit: Int) -> some View {
        VStack(alignment: .leading, spacing: large ? 14 : 9) {
            ForEach(Array(items.prefix(limit).enumerated()), id: \.element.id) { index, item in
                if index > 0 { Rectangle().fill(palette.border).frame(height: 1) }
                Button(intent: OpenNativeItemIntent(itemId: item.id, section: sectionKind, scope: snapshot.scope)) {
                    HStack(alignment: .top, spacing: 9) {
                        if sectionKind == "inbox" {
                            FlowNotificationIcon(item: item, scope: snapshot.scope, size: small || accessible ? 24 : 30)
                                .padding(.top, 1)
                        }
                        VStack(alignment: .leading, spacing: small ? 7 : 3) {
                            if sectionKind == "attention" {
                                HStack {
                                    statusLabel(item)
                                    if !small && !accessible {
                                        Spacer(minLength: 4)
                                        itemTime(item).font(.caption2).foregroundStyle(palette.secondary)
                                    }
                                }
                            }
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text(item.title).font(.subheadline.weight(.semibold))
                                    .lineLimit(accessible && sectionKind == "inbox" ? 3 : (small || large || accessible ? 2 : 1)).multilineTextAlignment(.leading)
                                    .fixedSize(horizontal: false, vertical: small || accessible)
                                if !small && !accessible {
                                    Spacer(minLength: 0)
                                    if sectionKind == "recent_runs" { statusLabel(item) }
                                }
                            }
                            if sectionKind == "recent_runs" && (small || accessible) { statusLabel(item) }
                            if sectionKind == "event_favorites" {
                                Label(item.action.kind == "run_event" ? "Run Event" : "Open page", systemImage: item.action.kind == "run_event" ? "play.fill" : "arrow.up.right")
                                    .font(.caption2).foregroundStyle(palette.accentText)
                            } else if !accessible {
                                if sectionKind == "inbox", let body = item.subtitle {
                                    Text(body).font(.caption).foregroundStyle(palette.secondary).lineLimit(small ? 2 : (large ? 3 : 1))
                                } else if sectionKind != "attention" || small { itemTime(item).font(.caption2).foregroundStyle(palette.secondary) }
                            }
                            if let progress = item.progress, item.status?.lowercased() == "running" {
                                runProgress(progress)
                            }
                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }.contentShape(Rectangle())
                }.buttonStyle(.plain).privacySensitive()
            }
        }
    }

    private func runProgress(_ value: Double) -> some View {
        let progress = max(0, min(1, value))
        return GeometryReader { geometry in
            ZStack(alignment: .leading) {
                Capsule().fill(palette.border)
                Capsule().fill(palette.accentText).frame(width: geometry.size.width * progress)
            }
        }.frame(height: 3).padding(.top, 3)
            .accessibilityLabel("Progress").accessibilityValue(progress.formatted(.percent.precision(.fractionLength(0))))
    }

    @ViewBuilder private func itemTime(_ item: NativeItem) -> some View {
        if let raw = sectionKind == "recent_apps" ? item.subtitle : item.startedAt,
           let date = NativeSnapshot.date(raw) {
            if Calendar.current.isDateInToday(date) { Text(date, style: .time) }
            else { Text(date, format: .dateTime.month(.abbreviated).day()) }
        }
    }

    private func statusLabel(_ item: NativeItem) -> some View {
        let status = item.status ?? item.subtitle ?? "Open run"
        let lower = status.lowercased()
        let error = lower == "fatal" || lower.contains("fail") || lower.contains("error")
        let warning = lower == "warn" || lower.contains("wait")
        let color = error ? Color(widgetHex: palette.dark ? 0xffa19a : 0xac3029) :
            warning ? Color(widgetHex: palette.dark ? 0xecc779 : 0x805d15) : palette.secondary
        return HStack(spacing: 5) {
            Circle().fill(color).frame(width: 5, height: 5)
            Text(status.capitalized).lineLimit(1)
        }.font(.caption2.weight(.medium)).foregroundStyle(color)
    }

    private var emptyContent: some View {
        VStack(alignment: .leading, spacing: 7) {
            Spacer(minLength: 0)
            if !small { Image(systemName: emptySymbol).font(.title2.weight(.light)).foregroundStyle(palette.secondary).accessibilityHidden(true) }
            Text(emptyTitle).font(.subheadline.weight(.semibold)).lineLimit(2)
            if !accessible { Text(emptyMessage).font(.caption).foregroundStyle(palette.secondary).lineLimit(small ? 2 : 3) }
            Spacer(minLength: 0)
            Link(destination: NativeWidgetLaunchURL.home) {
                Label(small && accessible ? "Open" : "Open Flow Like", systemImage: "arrow.up.right")
                    .font(.caption.weight(.medium)).foregroundStyle(palette.accentText)
            }
        }
    }

    private var emptySymbol: String { entry.snapshot == nil ? "arrow.clockwise" : section?.state == "signed_out" ? "person.crop.circle" : symbol }
    private var emptyTitle: String {
        if entry.snapshot == nil { return "Refresh widget" }
        if section?.state == "signed_out" { return "Sign in" }
        if section?.state == "unavailable" { return "Updates unavailable" }
        switch sectionKind {
        case "attention": return "No recent errors"
        case "inbox": return "No notifications"
        case "event_favorites": return "No favorites"
        case "recent_apps": return "No recent apps"
        case "recent_runs": return "No recent runs"
        default: return "No activity yet"
        }
    }
    private var emptyMessage: String {
        if entry.snapshot == nil { return "Open the app to update." }
        if section?.state == "signed_out" { return "Connect your workspace." }
        if section?.state == "unavailable" { return "Open Flow Like to reconnect." }
        switch sectionKind {
        case "event_favorites": return "Add favorites in Event settings."
        case "recent_apps": return "Apps you open will appear here."
        case "recent_runs": return "Runs will appear here."
        case "attention": return "Available run history is clear."
        default: return "Updates will appear here."
        }
    }
}

protocol NativeSectionWidget: Widget {
    static var sectionKind: String { get }
    static var title: String { get }
    static var symbol: String { get }
    static var families: [WidgetFamily] { get }
}

extension NativeSectionWidget {
    static var families: [WidgetFamily] { [.systemSmall, .systemMedium, .systemLarge] }
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "com.flow-like.app.\(Self.sectionKind)", provider: WorkspaceProvider()) { entry in
            WorkspaceWidgetView(entry: entry, sectionKind: Self.sectionKind, title: Self.title, symbol: Self.symbol)
        }
        .configurationDisplayName(Self.title)
        .description(Text(verbatim: "See \(Self.title.lowercased()) from your Flow Like workspace."))
        .supportedFamilies(Self.families)
    }
}

struct FlowPilotWidget: NativeSectionWidget {
    static let families: [WidgetFamily] = [.systemSmall, .systemMedium]
    static let sectionKind = "flowpilot"
    static let title = "FlowPilot"
    static let symbol = "sparkles"
}

struct InboxWidget: NativeSectionWidget {
    static let sectionKind = "inbox"
    static let title = "Notifications"
    static let symbol = "bell"
}

struct AttentionWidget: NativeSectionWidget {
    static let sectionKind = "attention"
    static let title = "Needs Attention"
    static let symbol = "exclamationmark.circle"
}

struct RecentRunsWidget: NativeSectionWidget {
    static let sectionKind = "recent_runs"
    static let title = "Recent Runs"
    static let symbol = "clock.arrow.circlepath"
}

struct RecentAppsWidget: NativeSectionWidget {
    static let sectionKind = "recent_apps"
    static let title = "Recent Apps"
    static let symbol = "square.grid.2x2"
}

struct UsageWidget: NativeSectionWidget {
    static let families: [WidgetFamily] = [.systemSmall, .systemMedium]
    static let sectionKind = "usage"
    static let title = "App Usage"
    static let symbol = "chart.bar"
}

struct WorkspaceWidget: NativeSectionWidget {
    static let sectionKind = "workspace"
    static let title = "Workspace Overview"
    static let symbol = "rectangle.3.group"
}

struct EventFavoritesWidget: NativeSectionWidget {
    static let sectionKind = "event_favorites"
    static let title = "Event Favorites"
    static let symbol = "star"
}

struct FlowOpenAppConfiguration: WidgetConfigurationIntent {
    static let title: LocalizedStringResource = "Open App"
    static let description = IntentDescription("Open an app at a chosen page with optional query parameters.")

    @Parameter(title: "App") var app: FlowAppEntity?
    @Parameter(title: "Internal path") var path: String?
    @Parameter(title: "Query parameter names") var queryNames: [String]?
    @Parameter(title: "Query parameter values") var queryValues: [String]?
    @Parameter(title: "Widget title") var widgetTitle: String?

    static var parameterSummary: some ParameterSummary {
        Summary("Open \(\.$app)") {
            \.$path
            \.$queryNames
            \.$queryValues
            \.$widgetTitle
        }
    }
}

struct OpenAppEntry: TimelineEntry {
    let date: Date
    let snapshot: NativeSnapshot?
    let configuration: FlowOpenAppConfiguration
}

struct OpenAppProvider: AppIntentTimelineProvider {
    func placeholder(in context: Context) -> OpenAppEntry {
        OpenAppEntry(date: Date(), snapshot: nil, configuration: FlowOpenAppConfiguration())
    }

    func snapshot(for configuration: FlowOpenAppConfiguration, in context: Context) async -> OpenAppEntry {
        OpenAppEntry(date: Date(), snapshot: NativeStore.shared.snapshot(), configuration: configuration)
    }

    func timeline(for configuration: FlowOpenAppConfiguration, in context: Context) async -> Timeline<OpenAppEntry> {
        let snapshot = NativeStore.shared.snapshot()
        let now = Date()
        var entries = [OpenAppEntry(date: now, snapshot: snapshot, configuration: configuration)]
        if let expiry = snapshot.flatMap({ NativeSnapshot.date($0.expiresAt) }), expiry > now {
            entries.append(OpenAppEntry(date: expiry, snapshot: nil, configuration: configuration))
        }
        return Timeline(entries: entries, policy: .after(now.addingTimeInterval(900)))
    }
}

struct OpenAppWidgetView: View {
    let entry: OpenAppEntry
    @Environment(\.widgetFamily) private var family
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    private var palette: FlowWidgetPalette { FlowWidgetPalette(dark: colorScheme == .dark) }
    private var small: Bool { family == .systemSmall }
    private var accessible: Bool { dynamicTypeSize.isAccessibilitySize }
    private var selection: NativeWidgetAppSelection {
        NativeWidgetAppSelection(snapshot: entry.snapshot, appId: entry.configuration.app?.sourceId,
                                 scope: entry.configuration.app?.scope, now: entry.date)
    }
    private var routeError: String? {
        do {
            let query = try NativeAppRoute.queryParameters(names: entry.configuration.queryNames,
                                                           values: entry.configuration.queryValues)
            try NativeAppRoute.validate(path: entry.configuration.path, queryParams: query)
            return nil
        } catch { return error.localizedDescription }
    }
    private var displayPath: String { NativeWidgetAppSelection.displayPath(entry.configuration.path) }

    var body: some View {
        Group {
            if let app = selection.app, let scope = entry.snapshot?.scope, routeError == nil {
                Button(intent: OpenFlowAppIntent(app: FlowAppEntity(app, scope: scope),
                                                path: entry.configuration.path,
                                                queryNames: entry.configuration.queryNames,
                                                queryValues: entry.configuration.queryValues)) {
                    destinationContent(app)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain).privacySensitive()
                .accessibilityLabel(destinationTitle(app) == app.title ? "Open \(app.title)" : "Open \(destinationTitle(app)) in \(app.title)")
                .accessibilityHint(displayPath)
                .widgetURL(try? NativeWidgetLaunchURL.app(appId: app.id, scope: scope,
                                                        path: entry.configuration.path,
                                                        queryNames: entry.configuration.queryNames,
                                                        queryValues: entry.configuration.queryValues))
            } else { unavailableContent }
        }
        .foregroundStyle(palette.ink)
        .containerBackground(for: .widget) { FlowWidgetBackground(kind: "open_app") }
    }

    private func destinationTitle(_ app: NativeApp) -> String {
        let custom = entry.configuration.widgetTitle?.trimmingCharacters(in: .whitespacesAndNewlines)
        return custom.flatMap { $0.isEmpty ? nil : $0 } ?? app.title
    }

    private var brand: some View {
        FlowLikeMark().fill(FlowWidgetPalette.gradient)
            .frame(width: 15, height: 17)
            .widgetAccentable().accessibilityHidden(true)
    }

    private var arrow: some View {
        Image(systemName: "arrow.up.right")
            .font(.system(size: small ? 13 : 18, weight: .medium))
            .foregroundStyle(palette.ink)
            .accessibilityHidden(true)
    }

    private func appBadge(_ app: NativeApp, size: CGFloat) -> some View {
        FlowAppIcon(scope: entry.snapshot?.scope ?? "", appId: app.id, title: app.title, size: size)
    }

    @ViewBuilder
    private func destinationContent(_ app: NativeApp) -> some View {
        if small { smallDestination(app) }
        else { mediumDestination(app) }
    }

    private func smallDestination(_ app: NativeApp) -> some View {
        VStack(alignment: .leading, spacing: accessible ? 5 : 8) {
            HStack(alignment: .top) {
                appBadge(app, size: accessible ? 23 : 30)
                Spacer(minLength: 8)
                brand
            }
            Spacer(minLength: 0)
            Text(destinationTitle(app))
                .font(accessible ? .subheadline.weight(.semibold) : .headline.weight(.semibold))
                .lineLimit(2).minimumScaleFactor(0.85).multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
            Spacer(minLength: 0)
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(displayPath).font(.caption2).foregroundStyle(palette.secondary)
                    .lineLimit(1).truncationMode(.middle)
                Spacer(minLength: 0)
                arrow
            }
        }
    }

    private func mediumDestination(_ app: NativeApp) -> some View {
        HStack(alignment: .center, spacing: accessible ? 12 : 16) {
            if !accessible { appBadge(app, size: 52) }
            VStack(alignment: .leading, spacing: accessible ? 4 : 7) {
                if destinationTitle(app) != app.title {
                    Text(app.title).font(.caption).foregroundStyle(palette.secondary)
                        .lineLimit(1)
                }
                Text(destinationTitle(app))
                    .font(accessible ? .headline.weight(.semibold) : .title3.weight(.semibold))
                    .lineLimit(2).minimumScaleFactor(0.85).multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Text(displayPath).font(.caption).foregroundStyle(palette.secondary)
                    .lineLimit(1).truncationMode(.middle)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            VStack(alignment: .trailing) {
                brand
                Spacer(minLength: 12)
                arrow
            }
            .frame(maxHeight: .infinity)
        }
        .frame(maxHeight: .infinity)
    }

    private var unavailableContent: some View {
        VStack(alignment: .leading, spacing: accessible ? 6 : 10) {
            HStack(alignment: .top) {
                if !accessible {
                    Image(systemName: selection.state == .chooseApp ? "app.badge" : "app.dashed")
                        .font(.system(size: small ? 23 : 28, weight: .regular))
                        .foregroundStyle(palette.secondary).accessibilityHidden(true)
                }
                Spacer(minLength: 8)
                brand
            }
            Spacer(minLength: 0)
            Text(unavailableTitle).font(.subheadline.weight(.semibold))
                .lineLimit(2).minimumScaleFactor(0.85)
                .frame(maxWidth: .infinity, alignment: .leading)
            if !accessible {
                Text(unavailableMessage).font(.caption).foregroundStyle(palette.secondary)
                    .lineLimit(small ? 2 : 3)
            }
            Spacer(minLength: 0)
            if selection.state == .refresh || selection.state == .unavailable {
                Link(destination: NativeWidgetLaunchURL.home) {
                    HStack {
                        Text(small && accessible ? "Refresh" : "Open Flow Like")
                            .font(.caption.weight(.medium)).lineLimit(1)
                        Spacer(minLength: 8)
                        arrow
                    }
                }
            } else {
                Text(accessible ? "Edit settings" : "Edit in widget settings").font(.caption2).foregroundStyle(palette.secondary)
                    .lineLimit(2)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .widgetURL(NativeWidgetLaunchURL.home)
    }

    private var unavailableTitle: String {
        switch selection.state {
        case .refresh: return "Refresh apps"
        case .chooseApp: return "Choose an app"
        case .unavailable: return "App unavailable"
        case .ready: return "Check the path"
        @unknown default: return "Refresh apps"
        }
    }

    private var unavailableMessage: String {
        switch selection.state {
        case .refresh: return "Update your app list."
        case .chooseApp: return "Set an app and page."
        case .unavailable: return "Choose an available app."
        case .ready: return "Check path and query settings."
        @unknown default: return "Update your app list."
        }
    }
}


struct OpenAppWidget: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(kind: "com.flow-like.app.open_app", intent: FlowOpenAppConfiguration.self,
                               provider: OpenAppProvider()) { entry in
            OpenAppWidgetView(entry: entry)
        }
        .configurationDisplayName("Open App")
        .description("Open an app, choose its page, and supply query parameters.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

#if os(iOS)
struct FlowRunActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: FlowRunAttributes.self) { context in
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    FlowLikeMark().fill(FlowWidgetPalette.gradient).frame(width: 20, height: 20).accessibilityHidden(true)
                    Text(context.state.title).font(.headline).lineLimit(1)
                }
                Text(context.isStale ? "Open Flow Like for current status" : context.state.status.capitalized)
                    .font(.caption).foregroundStyle(.secondary)
                if !context.isStale, let progress = context.state.progress { ProgressView(value: progress) }
            }.padding().privacySensitive()
                .widgetURL(runURL(context.attributes))
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) { FlowLikeMark().fill(FlowWidgetPalette.gradient).frame(width: 20, height: 20) }
                DynamicIslandExpandedRegion(.center) { Text(context.state.title).lineLimit(1).privacySensitive() }
                DynamicIslandExpandedRegion(.bottom) {
                    if context.isStale {
                        Text("Open Flow Like for current status").font(.caption).privacySensitive()
                    } else if let progress = context.state.progress { ProgressView(value: progress) }
                    else { Text(context.state.status.capitalized).font(.caption).privacySensitive() }
                }
            } compactLeading: { FlowLikeMark().fill(FlowWidgetPalette.gradient).frame(width: 16, height: 16) }
              compactTrailing: { Image(systemName: context.isStale ? "clock" : "arrow.trianglehead.2.clockwise.rotate.90") }
              minimal: { FlowLikeMark().fill(FlowWidgetPalette.gradient).frame(width: 16, height: 16) }
              .widgetURL(runURL(context.attributes))
        }
    }
    private func runURL(_ attributes: FlowRunAttributes) -> URL {
        NativeWidgetLaunchURL.run(appId: attributes.appId, runId: attributes.runId, scope: attributes.scope)
    }
}

#endif

@available(iOSApplicationExtension 18.0, macOSApplicationExtension 26.0, *)
struct FlowPilotControl: ControlWidget {
    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: "com.flow-like.app.flowpilot-control") {
            ControlWidgetButton(action: OpenFlowPilotIntent()) {
                Label("FlowPilot", systemImage: "sparkles")
            }
        }.displayName("FlowPilot").description("Open a FlowPilot conversation.")
    }
}

@available(iOSApplicationExtension 18.0, macOSApplicationExtension 26.0, *)
struct FlowInboxControl: ControlWidget {
    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: "com.flow-like.app.inbox-control") {
            ControlWidgetButton(action: OpenFlowInboxIntent()) {
                Label("Notifications", systemImage: "bell")
            }
        }.displayName("Notifications").description("Open Flow Like notifications.")
    }
}

@available(iOSApplicationExtension 18.0, macOSApplicationExtension 26.0, *)
struct FlowEventControl: ControlWidget {
    var body: some ControlWidgetConfiguration {
        AppIntentControlConfiguration(kind: "com.flow-like.app.event-control", intent: FlowEventControlConfiguration.self) { configuration in
            ControlWidgetButton(action: RunWidgetFlowEventIntent(event: configuration.event)) {
                Label(configuration.event?.title ?? "Choose Event", systemImage: "bolt.fill")
            }.disabled(configuration.event == nil)
        }.displayName("Event").description("Open a page or run a favorite Event in Flow Like.")
    }
}

@main
struct FlowLikeWidgetBundle: WidgetBundle {
    var body: some Widget {
        FlowPilotWidget()
        InboxWidget()
        AttentionWidget()
        RecentRunsWidget()
        RecentAppsWidget()
        UsageWidget()
        WorkspaceWidget()
        EventFavoritesWidget()
        OpenAppWidget()
        #if os(iOS)
        FlowRunActivityWidget()
        #endif
        if #available(iOSApplicationExtension 18.0, macOSApplicationExtension 26.0, *) {
            FlowPilotControl()
            FlowInboxControl()
            FlowEventControl()
        }
    }
}
