import SwiftUI
import UIKit

/// A long press on one item of the system tab bar. SwiftUI's `TabView` has no
/// hook for it, so this probe finds the `UITabBarController` the TabView is
/// built on and attaches a `UILongPressGestureRecognizer` to its `tabBar`.
/// Events carry window coordinates, which are SwiftUI's `.global` space.
struct TabBarLongPress: UIViewControllerRepresentable {
    enum Event {
        /// The press was recognised on the item; `item` is its frame.
        case began(item: CGRect, at: CGPoint)
        case changed(CGPoint)
        case ended(CGPoint)
        case cancelled
    }

    /// Index of the watched item among the tab bar's items.
    let index: Int
    var minimumDuration: TimeInterval = 0.35
    let onEvent: (Event) -> Void

    func makeUIViewController(context: Context) -> Probe {
        Probe()
    }

    func updateUIViewController(_ probe: Probe, context: Context) {
        probe.index = index
        probe.minimumDuration = minimumDuration
        probe.onEvent = onEvent
        probe.attach()
    }

    static func dismantleUIViewController(_ probe: Probe, coordinator: ()) {
        probe.detach()
    }

    final class Probe: UIViewController, UIGestureRecognizerDelegate {
        var index = 0
        var minimumDuration: TimeInterval = 0.35
        var onEvent: ((Event) -> Void)?
        private weak var tabBar: UITabBar?
        private var recognizer: UILongPressGestureRecognizer?
        /// Set when the press began on the watched item.
        private var tracking = false

        override func loadView() {
            view = UIView()
            view.isUserInteractionEnabled = false
        }

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            attach()
        }

        override func viewDidLayoutSubviews() {
            super.viewDidLayoutSubviews()
            attach()
        }

        /// Finds the tab bar: an ancestor's when the probe sits inside a tab,
        /// otherwise one under the window (the probe beside the TabView, so it
        /// works before the tab's page is ever loaded).
        func attach() {
            guard let bar = Self.tabBarController(from: self)?.tabBar ?? view.window.flatMap(Self.tabBar(in:))
            else { return }
            recognizer?.minimumPressDuration = minimumDuration
            guard bar !== tabBar else { return }
            detach()
            let press = UILongPressGestureRecognizer(target: self, action: #selector(handle(_:)))
            press.minimumPressDuration = minimumDuration
            press.delegate = self
            bar.addGestureRecognizer(press)
            tabBar = bar
            recognizer = press
        }

        func detach() {
            if let recognizer { tabBar?.removeGestureRecognizer(recognizer) }
            recognizer = nil
            tabBar = nil
        }

        private static func tabBarController(from probe: UIViewController) -> UITabBarController? {
            var node: UIViewController? = probe.parent
            while let current = node {
                if let tabs = current as? UITabBarController { return tabs }
                node = current.parent
            }
            guard let root = probe.view.window?.rootViewController else { return nil }
            return find(in: root)
        }

        private static func tabBar(in view: UIView) -> UITabBar? {
            if let bar = view as? UITabBar, !bar.isHidden { return bar }
            for child in view.subviews {
                if let bar = tabBar(in: child) { return bar }
            }
            return nil
        }

        private static func find(in controller: UIViewController) -> UITabBarController? {
            if let tabs = controller as? UITabBarController { return tabs }
            for child in controller.children {
                if let tabs = find(in: child) { return tabs }
            }
            return nil
        }

        /// Item frames in the tab bar, left to right. The item buttons are
        /// controls (iOS 18); failing that, the first group of sibling views
        /// that matches the item count side by side (the Liquid Glass platter
        /// of iOS 26 does not span the bar); failing both, the bar is split
        /// evenly by the item count.
        private func itemFrames(in bar: UITabBar) -> [CGRect] {
            let count = bar.items?.count ?? 0
            guard count > 0 else { return [] }
            var controls: [UIControl] = []
            Self.collect(bar, into: &controls)
            let visible = controls.filter(Self.isItemSized)
            if visible.count == count {
                return visible.map { $0.convert($0.bounds, to: bar) }.sorted { $0.minX < $1.minX }
            }
            if let row = Self.row(of: count, in: bar) {
                return row.map { $0.convert($0.bounds, to: bar) }.sorted { $0.minX < $1.minX }
            }
            let width = bar.bounds.width / CGFloat(count)
            return (0..<count).map { CGRect(x: CGFloat($0) * width, y: 0, width: width, height: bar.bounds.height) }
        }

        private static func isItemSized(_ view: UIView) -> Bool {
            !view.isHidden && view.alpha > 0.01 && view.bounds.width > 20
        }

        /// `count` visible siblings of equal width laid out left to right.
        private static func row(of count: Int, in view: UIView) -> [UIView]? {
            let children = view.subviews.filter(isItemSized)
            if children.count == count, let first = children.first,
                children.allSatisfy({ abs($0.frame.width - first.frame.width) < 2 && abs($0.frame.minY - first.frame.minY) < 2 }),
                Set(children.map { Int($0.frame.minX) }).count == count
            {
                return children
            }
            for child in view.subviews {
                if let found = row(of: count, in: child) { return found }
            }
            return nil
        }

        private static func collect(_ view: UIView, into controls: inout [UIControl]) {
            for child in view.subviews {
                if let control = child as? UIControl {
                    controls.append(control)
                } else {
                    collect(child, into: &controls)
                }
            }
        }

        @objc private func handle(_ press: UILongPressGestureRecognizer) {
            guard let bar = tabBar, let window = bar.window else { return }
            let point = press.location(in: window)
            switch press.state {
            case .began:
                let frames = itemFrames(in: bar)
                let local = press.location(in: bar)
                guard frames.indices.contains(index), frames[index].insetBy(dx: -4, dy: -8).contains(local) else {
                    tracking = false
                    return
                }
                tracking = true
                onEvent?(.began(item: bar.convert(frames[index], to: window), at: point))
            case .changed:
                if tracking { onEvent?(.changed(point)) }
            case .ended:
                if tracking { onEvent?(.ended(point)) }
                tracking = false
            case .cancelled, .failed:
                if tracking { onEvent?(.cancelled) }
                tracking = false
            default:
                break
            }
        }

        /// Only presses that start on the watched item; others stay the bar's.
        func gestureRecognizerShouldBegin(_ gesture: UIGestureRecognizer) -> Bool {
            guard let bar = tabBar else { return false }
            let frames = itemFrames(in: bar)
            guard frames.indices.contains(index) else { return false }
            return frames[index].insetBy(dx: -4, dy: -8).contains(gesture.location(in: bar))
        }

        func gestureRecognizer(
            _ gesture: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
        ) -> Bool {
            true
        }
    }
}
