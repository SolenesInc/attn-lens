// The points the menu bar takes at the top of a display, from NSScreen: its
// frame's top minus its visible frame's top. 0 when no screen has that id, or
// when the menu bar hides itself.
#import <AppKit/AppKit.h>

double lens_menu_bar_points(uint32_t display) {
    for (NSScreen *screen in NSScreen.screens) {
        NSNumber *number = screen.deviceDescription[@"NSScreenNumber"];
        if (number.unsignedIntValue == display) {
            return NSMaxY(screen.frame) - NSMaxY(screen.visibleFrame);
        }
    }
    return 0;
}
