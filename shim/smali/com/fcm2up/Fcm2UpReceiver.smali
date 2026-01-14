.class public final Lcom/fcm2up/Fcm2UpReceiver;
.super Landroid/content/BroadcastReceiver;
.source "SourceFile"


# annotations
.annotation runtime Lkotlin/Metadata;
    d1 = {
        "\u0000\u001c\n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0010\u0002\n\u0002\u0008\u0006\u0018\u0000 \n2\u00020\u0001:\u0001\u000bB\u0007\u00a2\u0006\u0004\u0008\u0008\u0010\tJ\u0018\u0010\u0007\u001a\u00020\u00062\u0006\u0010\u0003\u001a\u00020\u00022\u0006\u0010\u0005\u001a\u00020\u0004H\u0016\u00a8\u0006\u000c"
    }
    d2 = {
        "Lcom/fcm2up/Fcm2UpReceiver;",
        "Landroid/content/BroadcastReceiver;",
        "Landroid/content/Context;",
        "context",
        "Landroid/content/Intent;",
        "intent",
        "",
        "onReceive",
        "<init>",
        "()V",
        "Companion",
        "a/a",
        "fcm2up-shim_release"
    }
    k = 0x1
    mv = {
        0x1,
        0x9,
        0x0
    }
.end annotation


# static fields
.field public static final Companion:La/a;


# direct methods
.method static constructor <clinit>()V
    .registers 1

    new-instance v0, La/a;

    .line 1
    invoke-direct {v0}, La/a;-><init>()V

    .line 2
    sput-object v0, Lcom/fcm2up/Fcm2UpReceiver;->Companion:La/a;

    return-void
.end method

.method public constructor <init>()V
    .registers 1

    invoke-direct {p0}, Landroid/content/BroadcastReceiver;-><init>()V

    return-void
.end method


# virtual methods
.method public onReceive(Landroid/content/Context;Landroid/content/Intent;)V
    .registers 8

    const-string v0, "context"

    invoke-static {p1, v0}, Lkotlin/jvm/internal/Intrinsics;->checkNotNullParameter(Ljava/lang/Object;Ljava/lang/String;)V

    const-string v0, "intent"

    invoke-static {p2, v0}, Lkotlin/jvm/internal/Intrinsics;->checkNotNullParameter(Ljava/lang/Object;Ljava/lang/String;)V

    invoke-virtual {p2}, Landroid/content/Intent;->getAction()Ljava/lang/String;

    move-result-object v0

    if-nez v0, :cond_11

    return-void

    :cond_11
    const-string v1, "Received action: "

    invoke-virtual {v1, v0}, Ljava/lang/String;->concat(Ljava/lang/String;)Ljava/lang/String;

    move-result-object v1

    const-string v2, "FCM2UP"

    invoke-static {v2, v1}, Landroid/util/Log;->d(Ljava/lang/String;Ljava/lang/String;)I

    invoke-virtual {v0}, Ljava/lang/String;->hashCode()I

    move-result v1

    const v3, -0x52c601c5

    const-string v4, "message"

    if-eq v1, v3, :cond_71

    const v3, -0x1eb5b3f9

    if-eq v1, v3, :cond_64

    const v3, -0x18334929

    if-eq v1, v3, :cond_53

    const v3, 0x62b723a0

    if-eq v1, v3, :cond_38

    goto/16 :goto_a0

    :cond_38
    const-string v1, "org.unifiedpush.android.connector.NEW_ENDPOINT"

    invoke-virtual {v0, v1}, Ljava/lang/String;->equals(Ljava/lang/Object;)Z

    move-result v0

    if-nez v0, :cond_41

    goto :goto_a0

    .line 1
    :cond_41
    const-string v0, "endpoint"

    invoke-virtual {p2, v0}, Landroid/content/Intent;->getStringExtra(Ljava/lang/String;)Ljava/lang/String;

    move-result-object p2

    if-eqz p2, :cond_4d

    invoke-static {p1, p2}, Lcom/fcm2up/Fcm2UpShim;->onNewEndpoint(Landroid/content/Context;Ljava/lang/String;)V

    goto :goto_a0

    :cond_4d
    const-string p1, "NEW_ENDPOINT intent without endpoint"

    invoke-static {v2, p1}, Landroid/util/Log;->w(Ljava/lang/String;Ljava/lang/String;)I

    goto :goto_a0

    .line 2
    :cond_53
    const-string v1, "org.unifiedpush.android.connector.REGISTRATION_FAILED"

    invoke-virtual {v0, v1}, Ljava/lang/String;->equals(Ljava/lang/Object;)Z

    move-result v0

    if-nez v0, :cond_5c

    goto :goto_a0

    .line 3
    :cond_5c
    invoke-virtual {p2, v4}, Landroid/content/Intent;->getStringExtra(Ljava/lang/String;)Ljava/lang/String;

    move-result-object p2

    invoke-static {p1, p2}, Lcom/fcm2up/Fcm2UpShim;->onRegistrationFailed(Landroid/content/Context;Ljava/lang/String;)V

    goto :goto_a0

    .line 4
    :cond_64
    const-string p2, "org.unifiedpush.android.connector.UNREGISTERED"

    invoke-virtual {v0, p2}, Ljava/lang/String;->equals(Ljava/lang/Object;)Z

    move-result p2

    if-nez p2, :cond_6d

    goto :goto_a0

    .line 5
    :cond_6d
    invoke-static {p1}, Lcom/fcm2up/Fcm2UpShim;->onUnregistered(Landroid/content/Context;)V

    goto :goto_a0

    .line 6
    :cond_71
    const-string v1, "org.unifiedpush.android.connector.MESSAGE"

    invoke-virtual {v0, v1}, Ljava/lang/String;->equals(Ljava/lang/Object;)Z

    move-result v0

    if-nez v0, :cond_7a

    goto :goto_a0

    .line 7
    :cond_7a
    const-string v0, "bytesMessage"

    invoke-virtual {p2, v0}, Landroid/content/Intent;->getByteArrayExtra(Ljava/lang/String;)[B

    move-result-object v0

    if-eqz v0, :cond_86

    invoke-static {p1, v0}, Lcom/fcm2up/Fcm2UpShim;->onMessage(Landroid/content/Context;[B)V

    goto :goto_a0

    :cond_86
    invoke-virtual {p2, v4}, Landroid/content/Intent;->getStringExtra(Ljava/lang/String;)Ljava/lang/String;

    move-result-object p2

    if-eqz p2, :cond_9b

    sget-object v0, Lkotlin/text/Charsets;->UTF_8:Ljava/nio/charset/Charset;

    invoke-virtual {p2, v0}, Ljava/lang/String;->getBytes(Ljava/nio/charset/Charset;)[B

    move-result-object p2

    const-string v0, "getBytes(...)"

    invoke-static {p2, v0}, Lkotlin/jvm/internal/Intrinsics;->checkNotNullExpressionValue(Ljava/lang/Object;Ljava/lang/String;)V

    invoke-static {p1, p2}, Lcom/fcm2up/Fcm2UpShim;->onMessage(Landroid/content/Context;[B)V

    goto :goto_a0

    :cond_9b
    const-string p1, "MESSAGE intent without message data"

    invoke-static {v2, p1}, Landroid/util/Log;->w(Ljava/lang/String;Ljava/lang/String;)I

    :goto_a0
    return-void
.end method
