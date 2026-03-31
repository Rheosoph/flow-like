#include "bindings/bindings.h"
#import <Foundation/Foundation.h>

int main(int argc, char * argv[]) {
        // Bootstrap push-notification bridge before Tauri's event loop starts.
        // PushNotificationBridge (AppDelegate.swift) installs an observer for
        // UIApplicationDidFinishLaunchingNotification that injects remote-
        // notification methods into the runtime-created AppDelegate.
        Class bridge = NSClassFromString(@"PushNotificationBridge");
        if (bridge) {
                SEL sel = NSSelectorFromString(@"prepareForLaunch");
                if ([bridge respondsToSelector:sel]) {
                        IMP imp = [bridge methodForSelector:sel];
                        ((void (*)(id, SEL))imp)((id)bridge, sel);
                }
        }

        ffi::start_app();
        return 0;
}
