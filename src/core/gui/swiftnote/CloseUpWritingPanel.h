#pragma once

#include <cstdint>

#include <gtk/gtk.h>

class Control;
class XojPageView;

namespace swiftnote {

class SwiftShell;

/**
 * Close-up Writing Area (Scrble Ink / GoodNotes style docked zoomed writing box).
 *
 * Maps stylus/pen/mouse input coordinates directly to exact page coordinates inside
 * XojPageView::onButtonPressEvent / onMotionNotifyEvent / onButtonReleaseEvent —
 * guaranteeing ZERO offset after pen-up, identical pen pressure, stabilizer, eraser,
 * auto-return, and Blackboard ink inversion.
 */
class CloseUpWritingPanel {
public:
    CloseUpWritingPanel(SwiftShell* shell, Control* control);
    ~CloseUpWritingPanel();

    GtkWidget* getWidget() const { return rootWidget; }
    bool isVisible() const { return visible; }

    void setVisible(bool show);
    void toggle();
    void moveLeft();
    void moveRight();
    void nextLine();
    void setZoomFactor(double factor);
    void queueRedraw();

private:
    void buildUi();
    XojPageView* currentPageView() const;
    void clampFocusBox(double pageW, double pageH);
    void scheduleAutoAdvance();
    void cancelAutoAdvance();

    static gboolean onDraw(GtkWidget* widget, cairo_t* cr, gpointer userData);
    static gboolean onButtonPress(GtkWidget* widget, GdkEventButton* ev, gpointer userData);
    static gboolean onMotionNotify(GtkWidget* widget, GdkEventMotion* ev, gpointer userData);
    static gboolean onButtonRelease(GtkWidget* widget, GdkEventButton* ev, gpointer userData);

private:
    SwiftShell* shell = nullptr;
    Control* control = nullptr;

    GtkWidget* rootWidget = nullptr;
    GtkWidget* drawingArea = nullptr;
    GtkWidget* zoomLabel = nullptr;

    bool visible = false;
    double zoomFactor = 2.5;
    bool autoAdvance = true;
    double marginTriggerRatio = 0.78;

    // Focus box top-left in page coordinates (pt)
    double focusX = 36.0;
    double focusY = 72.0;

    bool strokeActive = false;
    bool panningActive = false;
    double panStartX = 0.0;
    double panStartY = 0.0;
    double panOriginFocusX = 0.0;
    double panOriginFocusY = 0.0;
    double maxStrokeWidgetX = 0.0;

    guint autoAdvanceTimerId = 0;
};

}  // namespace swiftnote
