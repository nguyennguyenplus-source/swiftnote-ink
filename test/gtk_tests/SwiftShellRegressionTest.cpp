#include <cstdio>
#include <cstring>
#include <memory>

#include <cairo.h>
#include <gdk/gdkkeysyms.h>
#include <gtk/gtk.h>

#include "control/Control.h"
#include "control/ToolHandler.h"
#include "control/jobs/XournalScheduler.h"
#include "control/tools/TextEditor.h"
#include "control/zoom/ZoomControl.h"
#include "gui/GladeSearchpath.h"
#include "gui/MainWindow.h"
#include "gui/PageView.h"
#include "gui/XournalView.h"
#include "gui/inputdevices/PositionInputData.h"
#include "gui/swiftnote/SwiftBridge.h"
#include "gui/swiftnote/SwiftShell.h"
#include "gui/widgets/XournalWidget.h"
#include "model/Document.h"
#include "util/Color.h"

#include "config-test.h"
#include "dialog/GtkTest.h"

namespace {

void pumpGtk(int maxIters = 40, int sleepMs = 5) {
    for (int i = 0; i < maxIters; ++i) {
        int guard = 0;
        while (gtk_events_pending() && ++guard < 200) {
            gtk_main_iteration_do(FALSE);
        }
        g_usleep(static_cast<gulong>(sleepMs) * 1000UL);
    }
}

size_t countPixelsMatching(cairo_surface_t* surf, uint8_t r, uint8_t g, uint8_t b, int tol = 12) {
    cairo_surface_flush(surf);
    const int w = cairo_image_surface_get_width(surf);
    const int h = cairo_image_surface_get_height(surf);
    const int stride = cairo_image_surface_get_stride(surf);
    const uint8_t* data = cairo_image_surface_get_data(surf);
    size_t cnt = 0;
    for (int y = 0; y < h; ++y) {
        const uint32_t* row = reinterpret_cast<const uint32_t*>(data + y * stride);
        for (int x = 0; x < w; ++x) {
            const uint32_t px = row[x];
            const int pr = static_cast<int>((px >> 16) & 0xFF);
            const int pg = static_cast<int>((px >> 8) & 0xFF);
            const int pb = static_cast<int>(px & 0xFF);
            if (std::abs(pr - r) <= tol && std::abs(pg - g) <= tol && std::abs(pb - b) <= tol) {
                ++cnt;
            }
        }
    }
    return cnt;
}

}  // namespace

class SwiftShellRegressionTest: public GtkTest {
    void runTest(GtkApplication* app) override {
        GladeSearchpath gladeSearchPath{};
        gladeSearchPath.addSearchDirectory(GET_UI_FOLDER);

        auto control = std::make_unique<Control>(G_APPLICATION(app), &gladeSearchPath);
        auto win = std::make_unique<MainWindow>(&gladeSearchPath, control.get(), app);
        control->initWindow(win.get(), /*disableAudio=*/true);
        win->populate(&gladeSearchPath);
        win->show(nullptr);

        // Open a blank new document & start scheduler just like XournalMain
        control->newFile();
        control->getScheduler()->start();
        pumpGtk(60, 10);

        // 1. Check bottom statusbar & searchToolbar visibility
        GtkWidget* statusbar = win->get("statusbar");
        GtkWidget* searchBar = win->get("searchBar");
        printf("[DIAG] statusbar visible=%d, searchBar visible=%d\n",
               statusbar ? gtk_widget_get_visible(statusbar) : -1,
               searchBar ? gtk_widget_get_visible(searchBar) : -1);
        EXPECT_FALSE(gtk_widget_get_visible(statusbar)) << "Legacy statusbar ('State PLACEHOLDER') must stay hidden!";
        EXPECT_FALSE(gtk_widget_get_visible(searchBar)) << "Legacy searchBar must stay hidden on startup!";

        // 2. Check SwiftBridge pen slot color alpha == 0xFF
        auto slots = swiftnote::SwiftBridge::getPenSlots();
        ASSERT_FALSE(slots.empty());
        printf("[DIAG] slot[0].color = 0x%08X (alpha=%u)\n", uint32_t(slots[0].color), slots[0].color.alpha);
        EXPECT_EQ(slots[0].color.alpha, 0xFFU) << "PenSlot Color must have opaque alpha 0xFF!";

        // 3. Check page buffer & paintPage at normal 1.5x zoom
        auto* xournal = win->getXournal();
        ASSERT_NE(xournal, nullptr);
        control->getZoomControl()->setZoomFitMode(false);
        control->getZoomControl()->setZoom(1.5);
        pumpGtk(30, 10);
        auto* pv = xournal->getViewFor(0);
        ASSERT_NE(pv, nullptr);
        pv->rerenderPage();
        pumpGtk(40, 10);

        cairo_surface_t* surf = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, 920, 1280);
        cairo_t* cr = cairo_create(surf);
        cairo_set_source_rgb(cr, 0.86, 0.86, 0.84);
        cairo_paint(cr);
        pv->paintPage(cr, nullptr);
        EXPECT_EQ(cairo_status(cr), CAIRO_STATUS_SUCCESS);
        EXPECT_GT(countPixelsMatching(surf, 255, 255, 255, 10), 10000U);

        // 4. Toggle Setsquare & verify it actually paints geometry pixels without corrupting Cairo!
        const size_t whiteBaseline = countPixelsMatching(surf, 255, 255, 255, 10);
        const bool setsqOn = control->toggleSetsquare();
        pumpGtk(20, 10);
        cairo_set_source_rgb(cr, 0.86, 0.86, 0.84);
        cairo_paint(cr);
        pv->paintPage(cr, nullptr);
        const size_t whiteAfterSetsq = countPixelsMatching(surf, 255, 255, 255, 10);
        const size_t setsqChangedPx = (whiteBaseline > whiteAfterSetsq) ? (whiteBaseline - whiteAfterSetsq) : 0;
        printf("[DIAG] after toggleSetsquare(%d): cr_status=%s hasSetsquare=%d changed_px=%zu\n",
               setsqOn ? 1 : 0, cairo_status_to_string(cairo_status(cr)), control->hasSetsquare() ? 1 : 0,
               setsqChangedPx);
        EXPECT_TRUE(setsqOn);
        EXPECT_TRUE(control->hasSetsquare());
        EXPECT_EQ(cairo_status(cr), CAIRO_STATUS_SUCCESS);
        EXPECT_GT(setsqChangedPx, 5000U) << "Setsquare did not render visible overlay!";

        // Toggle off Setsquare, toggle on Compass
        const bool compOn = control->toggleCompass();
        pumpGtk(20, 10);
        cairo_set_source_rgb(cr, 0.86, 0.86, 0.84);
        cairo_paint(cr);
        pv->paintPage(cr, nullptr);
        const size_t whiteAfterComp = countPixelsMatching(surf, 255, 255, 255, 10);
        const size_t compChangedPx = (whiteBaseline > whiteAfterComp) ? (whiteBaseline - whiteAfterComp) : 0;
        printf("[DIAG] after toggleCompass(%d): cr_status=%s hasCompass=%d changed_px=%zu\n",
               compOn ? 1 : 0, cairo_status_to_string(cairo_status(cr)), control->hasCompass() ? 1 : 0, compChangedPx);
        EXPECT_TRUE(compOn);
        EXPECT_TRUE(control->hasCompass());
        EXPECT_FALSE(control->hasSetsquare());
        EXPECT_GT(compChangedPx, 5000U) << "Compass did not render visible overlay!";
        control->toggleCompass();  // turn off
        EXPECT_FALSE(control->hasCompass());
        pumpGtk(10, 5);

        // 5. Test Text Box on paper via real mouse click + typing
        control->selectTool(TOOL_TEXT);
        pumpGtk(10, 5);
        PositionInputData clickPos{};
        clickPos.x = 120.0 * xournal->getZoom();
        clickPos.y = 160.0 * xournal->getZoom();
        clickPos.pressure = -1.0;
        clickPos.state = static_cast<GdkModifierType>(0);
        clickPos.timestamp = 100;
        clickPos.deviceId = DeviceId{nullptr};
        pv->onButtonPressEvent(clickPos);
        pv->onButtonReleaseEvent(clickPos);
        pumpGtk(10, 5);
        auto* te = xournal->getTextEditor();
        ASSERT_NE(te, nullptr);
        // Verify single-key shortcuts ('e', 'r', 't', '1', 'b') are suppressed while TextEditor is active
        const guint testKeys[] = {GDK_KEY_H, GDK_KEY_e, GDK_KEY_r, GDK_KEY_t, GDK_KEY_1, GDK_KEY_b};
        for (guint kv: testKeys) {
            GdkEventKey kev{};
            kev.type = GDK_KEY_PRESS;
            kev.keyval = kv;
            kev.state = 0;
            EXPECT_FALSE(win->getSwiftShell()->handleKeyPress(&kev))
                    << "SwiftShell shortcut must NOT hijack keys while TextEditor is active!";
        }
        EXPECT_EQ(control->getToolHandler()->getToolType(), TOOL_TEXT);
        cairo_set_source_rgb(cr, 0.86, 0.86, 0.84);
        cairo_paint(cr);
        pv->paintPage(cr, nullptr);
        EXPECT_EQ(cairo_status(cr), CAIRO_STATUS_SUCCESS);
        pv->endText();
        pumpGtk(15, 10);
        cairo_set_source_rgb(cr, 0.86, 0.86, 0.84);
        cairo_paint(cr);
        pv->paintPage(cr, nullptr);
        EXPECT_EQ(cairo_status(cr), CAIRO_STATUS_SUCCESS);
        EXPECT_GT(countPixelsMatching(surf, 255, 255, 255, 10), 10000U);

        cairo_destroy(cr);
        cairo_surface_destroy(surf);

        control->getScheduler()->stop();
        win.reset();
        control.reset();
    }
};

TEST_F(SwiftShellRegressionTest, CanvasSetsquareTextAndStatusbar) {}
